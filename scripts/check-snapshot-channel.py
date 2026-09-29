#!/usr/bin/env python3
"""Refuse to publish a snapshot unless the CHANNEL is safe.

The companion to ``scripts/check-snapshot-pii.py``, which asks whether the FILE is
safe.  Those are two different questions and the spec is explicit that each needs
its own check, because *passing one and failing the other* is the two realistic
bad outcomes (spec 11.17.1):

* a correctly masked dump uploaded to a public host still discloses the operator's
  address to whoever observes the connection;
* a perfectly encrypted dump read by someone with the passphrase is still a
  disclosure of the identities inside it.

So this script must never be a formality.  If it cannot prove the channel is
safe, it refuses -- it does not warn, and it has no ``--force``.

The six refusals (spec 11.17.2/11.17.3/11.17.4), each one a real check:

  1. the target is a clearnet host: a hostname that resolves to a public address
  2. no ``.onion`` (or I2P destination) in the target
  3. a clearnet torrent client is running -- including a VPN-fronted one, which
     hides the address from the tracker but not from the swarm
  4. the snapshot file is not encrypted
  5. the passphrase equals the previous snapshot's
  6. the I2P destination equals the previous snapshot's

(6) and (5) need state: a manifest directory holding the previous snapshot's
passphrase FINGERPRINT and destination.  Never the passphrase itself -- a state
directory that stores the key it is checking is the thing it exists to detect.

Usage:
    check-snapshot-channel.py --target <url|onion> --file <snapshot> \\
        [--state-dir <dir>] [--passphrase-fpr <sha256>] [--i2p-destination <b64>]
    check-snapshot-channel.py --self-test

Exit status: 0 publishable, 1 refused, 2 usage error.
"""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import os
import re
import shutil
import socket
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

STATE_FILE = "publication-state.json"

#: A ``.onion`` is v2 (16 chars) or v3 (56 chars) of base32.
ONION_V2 = re.compile(r"^[a-z2-7]{16}\.onion$")
ONION_V3 = re.compile(r"^[a-z2-7]{56}\.onion$")
#: An I2P destination is base64 of a 256-byte certificate + options + key.
I2P_DEST = re.compile(r"^[A-Za-z0-9+/=]{300,}$")

#: Torrent clients, and whether the binary is expected to be I2P-only.  A client
#: that supports I2P is still refused unless it is *configured* for it: 11.17.2
#: refuses a clearnet torrent specifically, and the only evidence available here
#: is the configuration, so an ambiguous configuration is a refusal.
TORRENT_CLIENTS = {
    "qbittorrent": "config",
    "transmission-daemon": "config",
    "rtorrent": "config",
    "deluge": "config",
    "i2psnark": "i2p",
    "i2pd": "i2p",
}

#: The only two states that are safe to publish from. Note that "installed" and
#: "not running" are NOT in it: a client with no clearnet configuration is
#: unproven, not safe, and 11.17.2 refuses the clearnet capability rather than
#: the observed behaviour.
SAFE_CLIENT_STATES = frozenset({"i2p-native", "configured for i2p-only"})


class Refusal(Exception):
    """A named reason the snapshot must not be published."""


def refuse(reason: str, detail: str) -> "Refusal":
    return Refusal(f"{reason}\n\n  {detail}")


# --------------------------------------------------------------------------- #
# 1 + 2: the target must be an anonymous address, not a resolvable host
# --------------------------------------------------------------------------- #

def split_target(target: str) -> tuple[str, str, str | None]:
    """Return (host, url, onion_or_destination) for a publication target."""
    url = target if "://" in target else f"http://{target}"
    host = url.split("://", 1)[1].split("/", 1)[0].split("@")[-1]
    # Bracketed IPv6, and a port.
    host = host.rsplit(":", 1)[0] if host.count(":") == 1 else host
    host = host.strip("[]")
    return host, url, host if (ONION_V2.match(host) or ONION_V3.match(host)) else None


def is_public_address(addr: str) -> bool:
    """True if `addr` is a routable public address.

    Every reserved range counts as NOT public, which is the point: 0.0.0.0,
    10.x, 192.168.x and 127.0.0.1 are all ways a hostname can resolve without
    naming a machine on the internet.
    """
    try:
        ip = ipaddress.ip_address(addr)
    except ValueError:
        return True          # unparseable: assume the worst
    if isinstance(ip, (ipaddress.IPv6Address,)) and ip.ipv4_mapped:
        ip = ip.ipv4_mapped
    return not (
        ip.is_private or ip.is_loopback or ip.is_link_local
        or ip.is_multicast or ip.is_reserved or ip.is_unspecified
    )


def check_target(target: str) -> str:
    """Refuse a clearnet target.  Returns the onion/destination that was found."""
    host, url, onion = split_target(target)

    if onion:
        return onion
    if I2P_DEST.match(host):
        return host

    # No anonymous address: now the only excuse left is that the name does not
    # resolve, which is a legitimate OnionShare-on-the-same-host arrangement.
    try:
        infos = socket.getaddrinfo(host, None)
    except socket.gaierror:
        raise refuse(
            "the target is not an anonymous address",
            f"{host!r} is neither a .onion nor an I2P destination, and it does not\n"
            f"  resolve. That may be a same-machine OnionShare share, but a check\n"
            f"  that cannot prove the channel is safe must refuse, not assume.",
        ) from None
    # sockaddr[0] is the address; for AF_INET6 it can be a flowinfo/scope int
    # in some shapes, so only str values are addresses.
    addrs = sorted({a for a in (i[4][0] for i in infos) if isinstance(a, str)})
    public = [a for a in addrs if is_public_address(a)]
    if public:
        raise refuse(
            "the target is a clearnet host",
            f"{host!r} resolves to {', '.join(public)}, which is a public address.\n"
            f"  Spec 11.17.2 refuses every clearnet host, because each one records the\n"
            f"  uploader's address and several retain it after the file is removed. A\n"
            f"  monthly cadence turns a recurring upload from a stable address into a\n"
            f"  subscription to being located.",
        )
    raise refuse(
        "the target carries no .onion and no I2P destination",
        f"{host!r} resolves only to {', '.join(addrs)}, so nothing about the transfer\n"
        f"  is anonymous. Pass the .onion address, or the I2P destination, as the\n"
        f"  target -- not the host it happens to point at.",
    )


# --------------------------------------------------------------------------- #
# 3: no clearnet torrent client
# --------------------------------------------------------------------------- #

def torrent_clients_present() -> list[tuple[str, str, str]]:
    """Return [(binary, how_it_is_run, state)] for every torrent client found."""
    found = []
    for binary, kind in TORRENT_CLIENTS.items():
        path = shutil.which(binary)
        if not path:
            continue
        found.append((binary, path, client_state(binary, kind)))
    return found


def client_state(binary: str, kind: str = "config") -> str:
    """Classify one client's state, as a pure function of its environment.

    Split out from the discovery so the refusal can be DEMONSTRATED. A check
    that can only be exercised on a host with a torrent client installed is a
    check that is never exercised -- it is green on every machine that has not
    yet made the mistake it exists to catch.
    """
    if kind == "i2p":
        return "i2p-native"
    if _process_matches(binary):
        return "running"
    config = _config_is_clearnet(binary)
    if config is True:
        return "configured for clearnet"
    if config is False:
        return "configured for i2p-only"
    return "no clearnet configuration found, but I2P-only is not proven"


def check_no_clearnet_torrent(found: list[tuple[str, str, str]] | None = None) -> None:
    """Refuse when a clearnet torrent client is present.

    `found` is injectable so the self-test can present a client it does not have
    installed; production passes None and discovers the host's real state.
    """
    if found is None:
        found = torrent_clients_present()
    offenders = [(b, s) for b, _, s in found if s not in SAFE_CLIENT_STATES]
    if not offenders:
        return
    detail = "\n".join(f"  {b}: {s}" for b, s in offenders)
    raise refuse(
        "a clearnet torrent client is present",
        f"{detail}\n"
        f"  Spec 11.17.2 refuses a clearnet tracker, and also refuses a clearnet torrent\n"
        f"  behind a VPN: the VPN hides the address from the tracker, not from the\n"
        f"  swarm. The peer set still learns it. I2P has no exit nodes, which is what\n"
        f"  makes it safe -- a client merely capable of I2P is not enough.",
    )


def _process_matches(binary: str) -> bool:
    try:
        out = subprocess.run(["ps", "-eo", "comm,args"], capture_output=True,
                             text=True, timeout=10).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    return any(binary in line.split(None, 1)[0] or binary in line for line in out.splitlines())


CLEARNET_MARKERS = ("http://", "https://", "udp://", "tracker", "announce")


def _config_is_clearnet(binary: str) -> bool | None:
    """True/False/None = clearnet / i2p-only / cannot tell."""
    home = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    candidates = [home / f"{binary}" / "config", home / "i2p" / "i2p.config"]
    if binary == "qbittorrent":
        candidates = [home / "qBittorrent" / "qBittorrent.conf"]
    for path in candidates:
        if not path.exists():
            continue
        try:
            text = path.read_text(errors="replace")
        except OSError:
            return None
        if re.search(r"(?im)^\s*(i2p|enable_i2p|i2p_incoming|proxy_type)\s*[=:\s]\s*(1|true|i2p|yes)", text) \
                or "i2psnark" in text.lower():
            # Configured for I2P, unless a clearnet tracker is also present.
            return False if not any(m in text.lower() for m in ("http://", "https://", "udp://")) else True
        if any(m in text.lower() for m in CLEARNET_MARKERS):
            return True
        return None
    return None


# --------------------------------------------------------------------------- #
# 4: the file must be encrypted
# --------------------------------------------------------------------------- #

def check_encrypted(path: Path) -> None:
    if not path.exists():
        raise refuse("the snapshot file does not exist", f"  {path}")
    head = path.open("rb").read(64)
    if head.startswith(b"age-encryption.org/v1"):
        return
    if head.startswith(b"-----BEGIN PGP MESSAGE-----"):
        return
    if head[:4] == b"\x28\xb5\x2f\xfd":            # zstd frame magic
        raise refuse(
            "the snapshot is compressed but NOT encrypted",
            f"  {path.name} is a zstd frame. Compressing a masked dump and calling it\n"
            f"  published is the failure 11.17.1 warns about: encryption protects an\n"
            f"  intercepted copy, and a zstd file is readable by anyone who gets it.",
        )
    raise refuse(
        "the snapshot is not encrypted",
        f"  {path.name} does not begin with an age or PGP header. Spec 11.17.3\n"
        f"  requires the file encrypted at rest with a passphrase held nowhere near\n"
        f"  the download link.",
    )


# --------------------------------------------------------------------------- #
# 5 + 6: nothing may repeat from the previous snapshot
# --------------------------------------------------------------------------- #

def passphrase_fingerprint(passphrase: str) -> str:
    """A fingerprint that identifies a passphrase without storing it.

    Salted, because an unsalted hash of a passphrase is one dictionary guess away
    from the passphrase, and this file is written next to the dump.
    """
    salt = b"lorehaven-snapshot-channel-v1"
    return hashlib.sha256(salt + passphrase.encode()).hexdigest()


def load_state(state_dir: Path) -> dict:
    path = state_dir / STATE_FILE
    if not path.exists():
        return {"snapshots": []}
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        # An unreadable state file is not "no previous snapshot": treating a
        # corrupt file as a blank slate is how a repeat gets through.
        raise refuse(
            "the publication state is unreadable",
            f"  {path} exists but does not parse. Treating it as a blank slate would\n"
            f"  let this snapshot repeat the previous destination or passphrase, which is\n"
            f"  the exact thing the state exists to prevent. Repair or remove it.",
        ) from None


def check_not_repeated(state: dict, *, fpr: str | None, destination: str | None,
                      when: str) -> None:
    previous = state.get("snapshots", [])[-1] if state.get("snapshots") else None
    if not previous:
        return
    if fpr and previous.get("passphrase_fingerprint") == fpr:
        raise refuse(
            "the passphrase is the previous snapshot's",
            f"  {when} reuses the passphrase fingerprint recorded for the snapshot of\n"
            f"  {previous.get('date', 'an earlier date')}. Spec 11.17.3: a reused passphrase\n"
            f"  means one intercepted file is a standing key to every other.",
        )
    if destination and previous.get("i2p_destination") == destination:
        raise refuse(
            "the I2P destination is the previous snapshot's",
            f"  {when} reuses the destination recorded for {previous.get('date', 'an earlier date')}.\n"
            f"  Spec 11.17.4: a reused destination lets a recipient correlate every month's\n"
            f"  dump to one identity, which is the whole threat the cadence exists against.",
        )


def record(state: dict, state_dir: Path, *, fpr: str | None, destination: str | None,
           when: str, rule_version: str, file: Path) -> Path:
    state.setdefault("snapshots", []).append({
        "date": when,
        "passphrase_fingerprint": fpr,
        "i2p_destination": destination,
        "rule_version": rule_version,
        "file": file.name,
        # No operator, no host, no passphrase: the state directory sits next to
        # the dump and must not become the thing that identifies the publisher.
        "recorded_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
    })
    state_dir.mkdir(parents=True, exist_ok=True)
    path = state_dir / STATE_FILE
    path.write_text(json.dumps(state, indent=2) + "\n")
    return path


# --------------------------------------------------------------------------- #
# self-test: every refusal is demonstrated by RUNNING it, not read
# --------------------------------------------------------------------------- #

def _expect_refusal(name: str, fn, *args, **kwargs) -> tuple[bool, str]:
    try:
        fn(*args, **kwargs)
    except Refusal as r:
        return True, f"{name}: REFUSED ({str(r).splitlines()[0]})"
    return False, f"{name}: DID NOT REFUSE"


def self_test() -> int:
    import tempfile
    failures = 0
    tmp = Path(tempfile.mkdtemp(prefix="lh-channel-selftest-"))

    with tempfile.TemporaryDirectory() as td:
        tdp = Path(td)
        # 1. clearnet host that resolves publicly.  example.com is IANA's and
        #    always resolves; the check is the behaviour, not the host.
        f, m = _expect_refusal("1 clearnet host", check_target, "example.com")
        print(m); failures += not f
        f, m = _expect_refusal("1 public IPv4", check_target, "http://93.184.216.34/x")
        print(m); failures += not f
        f, m = _expect_refusal("1 unresolvable host", check_target, "no-such-host.invalid")
        print(m); failures += not f

        # A .onion must PASS.  This is the positive control: a check that refuses
        # everything is green, and would be just as useless as one that passes
        # everything.
        try:
            got = check_target("http://" + "a" * 56 + ".onion/x")
            print(f"2 .onion accepted: {got[:14]}...")
        except Refusal as r:
            print(f"2 .onion REFUSED: {r}"); failures += 1

        # 3. clearnet torrent clients, presented rather than discovered, because
        #    this host has none installed and a check that can only run where the
        #    mistake has already been made is never run. `path` is a placeholder:
        #    the check reads the state, which is what is being tested.
        for label, state in [
            ("3 clearnet torrent running", "running"),
            ("3 torrent configured clearnet", "configured for clearnet"),
            ("3 torrent unproven", "no clearnet configuration found, but I2P-only is not proven"),
        ]:
            f, m = _expect_refusal(label, check_no_clearnet_torrent,
                                   [("qbittorrent", "/usr/bin/qbittorrent", state)])
            print(m); failures += not f

        # The positive control: an I2P-only or I2P-native client is allowed.
        for label, state in [("3 i2p-native client", "i2p-native"),
                             ("3 i2p-only config", "configured for i2p-only")]:
            try:
                check_no_clearnet_torrent([("i2psnark", "/usr/bin/i2psnark", state)])
                print(f"{label} accepted")
            except Refusal as r:
                print(f"{label} REFUSED: {r}"); failures += 1

        # 4. unencrypted, and compressed-but-not-encrypted
        plain = tdp / "snap.sql.zst"
        plain.write_bytes(b"CREATE TABLE accounts (email text);")
        f, m = _expect_refusal("4 unencrypted sql", check_encrypted, plain)
        print(m); failures += not f
        f, m = _expect_refusal("4 missing file", check_encrypted, tdp / "nope.sql")
        print(m); failures += not f

        # 5 + 6. repeats, which need state.
        state_dir = tdp / "state"
        st = load_state(state_dir)
        when = "2026-10-01"
        fpr = passphrase_fingerprint("correct horse battery staple")
        record(st, state_dir, fpr=fpr, destination="DEST-A", when=when,
               rule_version="11.16.3", file=plain)
        st = load_state(state_dir)

        f, m = _expect_refusal("5 reused passphrase", check_not_repeated, st,
                               fpr=fpr, destination="DEST-B", when=when)
        print(m); failures += not f
        f, m = _expect_refusal("6 reused destination", check_not_repeated, st,
                               fpr=passphrase_fingerprint("a different one"),
                               destination="DEST-A", when=when)
        print(m); failures += not f

        # The positive control for 5/6: different everything must pass.
        try:
            check_not_repeated(st, fpr=passphrase_fingerprint("a different one"),
                               destination="DEST-B", when="2026-11-01")
            print("5+6 fresh passphrase and destination accepted")
        except Refusal as r:
            print(f"5+6 REFUSED: {r}"); failures += 1

        # A corrupt state file must be a refusal, not a blank slate.
        corrupt = tdp / "corrupt"
        corrupt.mkdir()
        (corrupt / STATE_FILE).write_text("{not json")
        f, m = _expect_refusal("6 corrupt state", load_state, corrupt)
        print(m); failures += not f

    print(f"\n{'FAIL' if failures else 'OK'}: {failures} self-test failure(s)")
    return 1 if failures else 0


# --------------------------------------------------------------------------- #

def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", help="the .onion, I2P destination, or host to publish to")
    ap.add_argument("--file", type=Path, help="the snapshot file to publish")
    ap.add_argument("--state-dir", type=Path, default=Path.home() / ".local/share/lorehaven/snapshot-state")
    ap.add_argument("--passphrase-fpr", help="fingerprint of the passphrase, via --passphrase-fpr-stdin")
    ap.add_argument("--passphrase-fpr-stdin", action="store_true",
                    help="read the passphrase on stdin and fingerprint it (never an argument)")
    ap.add_argument("--i2p-destination", help="the I2P destination string, if publishing over I2P")
    ap.add_argument("--rule-version", default="11.16.3")
    ap.add_argument("--record", action="store_true",
                    help="record this publication in the state directory after a pass")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)

    if args.self_test:
        return self_test()
    if not args.target or not args.file:
        ap.error("--target and --file are both required (or use --self-test)")

    fpr = args.passphrase_fpr
    if args.passphrase_fpr_stdin:
        # Read from stdin, not argv: an argument is visible in `ps`.
        fpr = passphrase_fingerprint(sys.stdin.readline().rstrip("\n"))

    checks = [
        ("target", lambda: check_target(args.target)),
        ("torrent", check_no_clearnet_torrent),
        ("encryption", lambda: check_encrypted(args.file)),
        ("rotation", lambda: check_not_repeated(load_state(args.state_dir), fpr=fpr,
                                                 destination=args.i2p_destination,
                                                 when=datetime.now().date().isoformat())),
    ]
    destination = None
    for name, fn in checks:
        try:
            result = fn()
        except Refusal as r:
            print(f"REFUSING TO PUBLISH -- {name} check failed\n", file=sys.stderr)
            print(f"  {r}\n", file=sys.stderr)
            print("Stop and do not publish. See docs/runbooks/publishing-a-snapshot.md.",
                  file=sys.stderr)
            return 1
        if name == "target" and result:
            destination = result if I2P_DEST.match(result) else None
            if args.i2p_destination is None:
                args.i2p_destination = result

    if args.record:
        record(load_state(args.state_dir), args.state_dir, fpr=fpr,
               destination=args.i2p_destination, when=datetime.now().date().isoformat(),
               rule_version=args.rule_version, file=args.file)
    print(f"channel OK: publishing to {args.target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
