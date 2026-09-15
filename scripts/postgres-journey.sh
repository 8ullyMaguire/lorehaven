#!/usr/bin/env bash
#
# Drive every library endpoint against a live PostgreSQL server.
#
# Why this exists: this tree's tests run on SQLite. SQLite accepts `?` for a
# placeholder, casts a bigint straight to a boolean, compares a uuid to a text
# without complaint and sums integers to an integer — so a query that is wrong for
# PostgreSQL passes every test in the suite. Four defects in milestone 8's own new
# code were found only when this ran (see `docs/verification.md`, *Milestone 8*),
# and each is a class rather than an incident.
#
# It is not part of `cargo test` because it needs a server. It is here so that
# "PostgreSQL accepted the SQL" is a claim somebody can check rather than a claim
# somebody made once.
#
# Usage, with the server of your choice:
#
#   DATABASE_URL='postgres://user:pw@127.0.0.1:5432/db' ./scripts/postgres-journey.sh
#
# `PSQL` is the command used to seed rows; override it when the database is not
# reachable from a local `psql` (the default assumes the scratch container this was
# developed against). The script drops and recreates the `public` schema, so point
# it only at a scratch database.
set -uo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BIN=${BIN:-"$REPO_ROOT/target/debug/lorehaven"}
[ -x "$BIN" ] || BIN="$HOME/.cargo-target/debug/lorehaven"

DATABASE_URL=${DATABASE_URL:-'postgres://lorehaven:devpassword@127.0.0.1:55432/lorehaven'}
PSQL=${PSQL:-'sudo docker exec -i lh-m8-pg psql -U lorehaven -d lorehaven'}
PORT=${PORT:-8140}
ROOT=${ROOT:-/tmp/lh-postgres-journey}
LOG=${LOG:-/tmp/lh-postgres-journey.log}
JAR=/tmp/lh-postgres-journey-cookies
CONFIG=/tmp/lh-postgres-journey.toml
EMAIL='pg@lorehaven.local'
PASSWORD='a-long-enough-passphrase'
FAILED=0

[ -x "$BIN" ] || { echo "no binary at $BIN; set BIN=" >&2; exit 1; }

rm -rf "$ROOT"; mkdir -p "$ROOT"; rm -f "$JAR"

# A clean schema each run: the migrations are part of what is under test, and a
# leftover account would turn the registration below into a validation error.
$PSQL -q -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;' >/dev/null 2>&1 || true

# The default rate limits allow twenty writes in a burst, and this makes about
# twenty; a journey that trips its own limiter tests the limiter.
cat >"$CONFIG" <<'TOML'
[rate_limits]
auth_burst = 200
auth_per_minute = 2000
write_burst = 200
write_per_minute = 2000
search_burst = 200
search_per_minute = 2000
default_burst = 500
default_per_minute = 5000
TOML

"$BIN" --storage-root "$ROOT" --database-url "$DATABASE_URL" --port "$PORT" \
  --config "$CONFIG" serve >"$LOG" 2>&1 &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null' EXIT
echo "server pid $SERVER_PID"

# Ask `/health/ready` for its `status`, not any path for a 2xx: `/api/v1/health`
# falls through to the single-page app and answers 200 with HTML.
for _ in $(seq 1 120); do
  curl -fsS "http://127.0.0.1:$PORT/health/ready" 2>/dev/null | grep -q '"status":"ready"' && break
  sleep 0.5
done
curl -fsS "http://127.0.0.1:$PORT/health/ready" 2>/dev/null | grep -q '"status":"ready"' || {
  echo "server did not come up"; tail -30 "$LOG"; exit 1; }
echo "server up"

BASE="http://127.0.0.1:$PORT/api/v1"
JSON='content-type: application/json'

# step NAME EXPECTED_STATUS CURL_ARGS...
step() {
  local name=$1 expected=$2; shift 2
  local code
  code=$(curl -sS -m 25 -o /tmp/lh-step.json -w '%{http_code}' -b "$JAR" -c "$JAR" \
    -H "$AUTH" -H "$JSON" "$@")
  if [ "$code" = "$expected" ]; then
    printf '  %-46s %s\n' "$name" "$code"
  else
    printf '  %-46s %s (expected %s)\n' "$name" "$code" "$expected"
    head -c 400 /tmp/lh-step.json; echo
    FAILED=$((FAILED + 1))
  fi
}

# Registration and login are checked, not assumed: `curl -f` on a 422 turns a
# failed setup into thirty-seven silent 401s that look like an auth defect.
REG=$(curl -sS -m 25 -o /tmp/lh-reg.json -w '%{http_code}' -X POST -H "$JSON" \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\",\"handle\":\"pguser\",\"display_name\":\"PG\",\"age_band\":\"adult\"}" \
  "$BASE/auth/register")
[ "$REG" = "201" ] || { echo "registration failed with $REG:"; cat /tmp/lh-reg.json; exit 1; }

LOGIN=$(curl -sS -m 25 -c "$JAR" -o /tmp/lh-login.json -w '%{http_code}' -X POST -H "$JSON" \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" "$BASE/auth/login")
[ "$LOGIN" = "200" ] || { echo "login failed with $LOGIN:"; cat /tmp/lh-login.json; exit 1; }

CSRF=$(grep -i lorehaven_csrf "$JAR" | awk '{print $NF}')
[ -n "$CSRF" ] || { echo "no csrf cookie in the jar"; cat "$JAR"; exit 1; }
AUTH="x-csrf-token: $CSRF"
ACCOUNT=$($PSQL -tAc "SELECT id FROM accounts WHERE email = '$EMAIL' LIMIT 1;" | tr -d '[:space:]')
echo "account $ACCOUNT"

seed_item() {
  local n=$1 source=$2
  # `head -1`: psql prints the RETURNING row and then the command tag, and gluing
  # them together produces an identifier no column will accept.
  $PSQL -tAc "INSERT INTO library_items
    (id, account_id, work_id, source_key, source_work_key, title, author_text, author_url,
     summary, language, word_count, status, source_url, source_updated_at, last_synced_at,
     provenance_json, created_at, updated_at, version)
    VALUES (gen_random_uuid(), '$ACCOUNT', NULL, '$source', '$n', 'Item $n', 'Author', 'https://x/$n',
     'Summary $n', 'en', $((1000 * n)), 'ongoing', 'https://x/$n/story',
     '2026-09-0${n}T00:00:00Z', '2026-09-09T00:00:00Z', '{\"via\":\"api\"}',
     '2026-09-0${n}T00:00:00Z', '2026-09-09T00:00:00Z', 1)
    RETURNING id;" | head -1 | tr -d '[:space:]'
}
ITEM1=$(seed_item 1 royalroad)
ITEM2=$(seed_item 2 ao3)
ITEM3=$(seed_item 3 royalroad)
for id in "$ITEM1" "$ITEM2" "$ITEM3"; do
  case "$id" in
    *-*-*-*-*) : ;;
    *) echo "seeding failed: '$id' is not an identifier"; exit 1 ;;
  esac
done
echo "items $ITEM1 $ITEM2 $ITEM3"

echo "--- shelves ---"
step "create a shelf"                        201 -X POST -d '{"name":"Favourites","description":"the good ones"}' "$BASE/shelves"
SHELF=$(python3 -c 'import json;print(json.load(open("/tmp/lh-step.json"))["id"])')
step "list shelves"                          200 "$BASE/shelves"
step "read one shelf"                        200 "$BASE/shelves/$SHELF"
step "put an item on it"                     204 -X POST "$BASE/shelves/$SHELF/items/$ITEM1"
step "put a second item on it"               204 -X POST "$BASE/shelves/$SHELF/items/$ITEM2"
step "rename it"                             204 -X PATCH -d '{"name":"Favourites","is_public":true,"expected_version":1}' "$BASE/shelves/$SHELF"
step "refuse a stale rename"                 409 -X PATCH -d '{"name":"No","expected_version":1}' "$BASE/shelves/$SHELF"
step "take an item off it"                   204 -X DELETE "$BASE/shelves/$SHELF/items/$ITEM2"

echo "--- private tags ---"
step "tag an item"                           204 -X PUT "$BASE/library/items/$ITEM1/tags/hold-for-winter"
step "read its tags"                         200 "$BASE/library/items/$ITEM1/tags"
step "tag a second item"                     204 -X PUT "$BASE/library/items/$ITEM2/tags/anthology"
step "untag an item"                         204 -X DELETE "$BASE/library/items/$ITEM2/tags/anthology"
step "refuse an empty tag"                   422 -X PUT "$BASE/library/items/$ITEM1/tags/%20"

echo "--- reading status ---"
step "set a status"                          200 -X PUT -d '{"status":"reading"}' "$BASE/library/items/$ITEM1/status"
step "read it back"                          200 "$BASE/library/items/$ITEM1/status"
step "move it on"                            200 -X PUT -d '{"status":"finished"}' "$BASE/library/items/$ITEM1/status"
step "refuse an unknown status"              422 -X PUT -d '{"status":"vibing"}' "$BASE/library/items/$ITEM1/status"

echo "--- bookmarks ---"
step "bookmark a work"                       201 -X POST -d "{\"subject_type\":\"work\",\"subject_id\":\"$ITEM1\",\"note\":\"the bit with the bridge\",\"position_permille\":420}" "$BASE/bookmarks"
BOOKMARK=$(python3 -c 'import json;print(json.load(open("/tmp/lh-step.json"))["id"])')
step "list them"                             200 "$BASE/bookmarks"
step "read one"                              200 "$BASE/bookmarks/$BOOKMARK"
step "edit the note"                         204 -X PATCH -d '{"note":"the bit with the bridge, again","expected_version":1}' "$BASE/bookmarks/$BOOKMARK"
step "refuse a stale edit"                   409 -X PATCH -d '{"note":"no","expected_version":1}' "$BASE/bookmarks/$BOOKMARK"

echo "--- saved views ---"
step "save a view"                           201 -X POST -d '{"name":"Unread, long","query":{"shelves":["Favourites"],"statuses":["reading"],"sort":"words"},"sort":"words","pinned":true}' "$BASE/saved-views"
VIEW=$(python3 -c 'import json;print(json.load(open("/tmp/lh-step.json"))["id"])')
step "list views"                            200 "$BASE/saved-views"
step "read one"                              200 "$BASE/saved-views/$VIEW"
step "refuse a public view with a shelf"     422 -X POST -d '{"name":"Leaky","query":{"shelves":["Favourites"]},"scope":"public"}' "$BASE/saved-views"
step "refuse a public view with a tag"       422 -X POST -d '{"name":"Leaky","query":{"tags":["hold-for-winter"]},"scope":"public"}' "$BASE/saved-views"

echo "--- listing and filters ---"
step "list, unfiltered"                      200 "$BASE/library/items"
step "filter by shelf"                       200 "$BASE/library/items?shelves=Favourites"
step "filter by tag"                         200 "$BASE/library/items?tags=hold-for-winter"
step "filter by status"                      200 "$BASE/library/items?statuses=finished"
step "filter by source"                      200 "$BASE/library/items?source=royalroad"
step "filter by updated since"               200 "$BASE/library/items?updated_since=2026-09-01T00:00:00Z"
step "sort by words"                         200 "$BASE/library/items?sort=words"
step "sort by updated"                       200 "$BASE/library/items?sort=updated"
step "sort by position"                      200 "$BASE/library/items?sort=position"
step "every filter at once"                  200 "$BASE/library/items?shelves=Favourites&tags=hold-for-winter&statuses=finished&source=royalroad&updated_since=2026-09-01T00:00:00Z&sort=words&limit=10"
step "page with a cursor"                    200 "$BASE/library/items?limit=1"

echo "--- storage and updates ---"
step "storage usage"                         200 "$BASE/library/storage"
step "queue an update check"                 202 -X POST "$BASE/library/updates/check"

echo "--- batch removal ---"
step "report an unknown id rather than fail" 200 -X POST -d "{\"ids\":[\"$ITEM2\",\"00000000-0000-4000-8000-000000000000\"],\"delete_copy\":false}" "$BASE/library/items/batch"

# ---------------------------------------------------------------------------
# Golden paths beyond the library: auth, authoring, monetization, community,
# notifications. A second account (the author) owns the work the first
# account (the buyer) purchases, so the money and notification flows can be
# observed from both sides.
# ---------------------------------------------------------------------------

echo "--- auth sanity ---"
step "read the session back"                   200 "$BASE/auth/me"
step "list sessions"                           200 "$BASE/auth/sessions"

BUYER_JAR="$JAR"; BUYER_AUTH="$AUTH"

AUTHOR_JAR=/tmp/lh-postgres-journey-author-cookies
rm -f "$AUTHOR_JAR"
A_REG=$(curl -sS -m 25 -o /tmp/lh-areg.json -w '%{http_code}' -X POST -H "$JSON" \
  -d '{"email":"pg-author@lorehaven.local","password":"a-long-enough-passphrase","handle":"pgauthor","display_name":"PG Author","age_band":"adult"}' \
  "$BASE/auth/register")
[ "$A_REG" = "201" ] || { echo "author registration failed with $A_REG:"; cat /tmp/lh-areg.json; exit 1; }
A_LOGIN=$(curl -sS -m 25 -c "$AUTHOR_JAR" -o /tmp/lh-alogin.json -w '%{http_code}' -X POST -H "$JSON" \
  -d '{"email":"pg-author@lorehaven.local","password":"a-long-enough-passphrase"}' "$BASE/auth/login")
[ "$A_LOGIN" = "200" ] || { echo "author login failed with $A_LOGIN:"; cat /tmp/lh-alogin.json; exit 1; }
AUTHOR_CSRF=$(grep -i lorehaven_csrf "$AUTHOR_JAR" | awk '{print $NF}')
AUTHOR_AUTH="x-csrf-token: $AUTHOR_CSRF"

as_author() { JAR="$AUTHOR_JAR"; AUTH="$AUTHOR_AUTH"; }
as_buyer()  { JAR="$BUYER_JAR";  AUTH="$BUYER_AUTH"; }

# expect_json NAME PYTHON_EXPR — assert a fact about the last step's response
# body. Run it immediately after the step it inspects: step() overwrites
# /tmp/lh-step.json every call.
expect_json() {
  if python3 -c "import json,sys; d=json.load(open('/tmp/lh-step.json')); d = d if isinstance(d, dict) else {}; sys.exit(0 if ($2) else 1)"; then
    printf '  %-46s ok\n' "$1"
  else
    printf '  %-46s FAIL (%s)\n' "$1" "$2"; head -c 300 /tmp/lh-step.json; echo
    FAILED=$((FAILED + 1))
  fi
}

echo "--- authoring a work ---"
as_author
step "create a work"                           201 -X POST -d '{"title":"The Bridge Book"}' "$BASE/works"
WORK=$(python3 -c 'import json;print(json.load(open("/tmp/lh-step.json"))["id"])')
step "add a chapter"                           201 -X POST -d '{"title":"One"}' "$BASE/works/$WORK/chapters"
CHAPTER=$(python3 -c 'import json;print(json.load(open("/tmp/lh-step.json"))["id"])')
step "write the chapter"                       200 -X PATCH -d '{"expected_version":1,"document":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"It was a dark and stormy night."}]}]}}' "$BASE/chapters/$CHAPTER"
step "publish the work"                        200 -X POST -d '{"expected_version":1}' "$BASE/works/$WORK/publish"
step "read it back as its author"              200 "$BASE/works/$WORK"
step "price it for purchase"                   200 -X POST -d '{"model":"purchase","price_minor":500,"currency":"EUR"}' "$BASE/works/$WORK/pricing"
step "refuse pricing from a non-owner"         404 -X POST -d '{"model":"purchase","price_minor":1,"currency":"EUR"}' "$BASE/works/$WORK/pricing" -b "$BUYER_JAR" -c "$BUYER_JAR" -H "$BUYER_AUTH"

as_buyer
step "a stranger cannot read the paywalled work" 403 "$BASE/works/$WORK"
step "purchase the work"                       200 -X POST -d '{}' "$BASE/works/$WORK/purchase"
step "purchase again is already granted"       200 -X POST -d '{}' "$BASE/works/$WORK/purchase"
expect_json "second purchase says already_granted" "d.get('status') == 'already_granted'"
step "list entitlements"                       200 "$BASE/me/entitlements"
expect_json "entitlement is bound to the work" "any(e['work_id'] == '$WORK' for e in d.get('entitlements', []))"
step "leave a review"                          200 -X PUT -d '{"body":"worth the money","contains_spoilers":false,"is_public":true}' "$BASE/works/$WORK/reviews"
step "tip the author"                          200 -X POST -d '{"amount_minor":100,"currency":"EUR","channel":"money"}' "$BASE/works/$WORK/tips"

echo "--- community ---"
as_author
CAT='33333333-3333-3333-3333-333333333333'
$PSQL -q -c "INSERT INTO forum_categories (id, name, position, min_trust) VALUES ('$CAT', 'Journey', 0, 0) ON CONFLICT (id) DO NOTHING;" >/dev/null 2>&1
step "create a forum topic"                    200 -X POST -d '{"title":"Hello from the journey"}' "$BASE/forums/$CAT/topics"
TOPIC=$(python3 -c 'import json;print(json.load(open("/tmp/lh-step.json"))["id"])')
step "read the topic"                          200 "$BASE/topics/$TOPIC"
step "list forums"                             200 "$BASE/forums"
step "list groups"                             200 "$BASE/groups"
as_buyer
step "reply to the topic"                      200 -X POST -d '{"body":"consider it replied"}' "$BASE/topics/$TOPIC/replies"
as_author
step "the author inbox holds the reply"        200 "$BASE/notifications"
expect_json "reply notification is unread"     "d.get('unread_count', 0) >= 1 and any(n['kind'] == 'reply' for n in d.get('items', []))"

echo "--- monetization notifications and earnings ---"
as_author
step "read author earnings"                    200 "$BASE/me/earnings"
step "the author inbox holds the sale"         200 "$BASE/notifications"
expect_json "sale notification arrived"        "any(n['kind'] == 'sale' and n.get('work_id') == '$WORK' for n in d.get('items', []))"
step "mark everything read"                    204 -X POST -d '{}' "$BASE/notifications/read-all"
step "the inbox settles to zero"               200 "$BASE/notifications"
expect_json "unread count is zero"             "d.get('unread_count', -1) == 0"
as_buyer
step "buyer inbox stayed empty"                200 "$BASE/notifications"
expect_json "no self-notifications"            "d.get('unread_count', -1) == 0"

echo "--- PostgreSQL's own complaints ---"
if grep -iE "mismatched types|is not compatible|operator does not exist|cannot cast|syntax error|invalid input syntax" "$LOG" >/tmp/lh-pg-errors.txt; then
  echo "FOUND:"; head -20 /tmp/lh-pg-errors.txt; FAILED=$((FAILED + 1))
else
  echo "  none"
fi
echo "failed steps: $FAILED"
exit $FAILED
