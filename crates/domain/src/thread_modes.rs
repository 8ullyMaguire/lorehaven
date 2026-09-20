/// Thread modes for creative work (spec §35.3, repo M33).
///
/// Each mode restructures one surface of a topic. `Plain` is the default and
/// behaves exactly as a topic did before modes existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadMode {
    /// A plain topic: no restructuring, the default.
    Plain,
    /// AMA / Q&A: questions float to the top; the author's replies render as
    /// highlighted cards.
    Ama,
    /// Reading group: a schedule of sections, each unlocking on a date.
    ReadingGroup,
    /// Critique circle: private group thread; members post excerpts on a turn
    /// queue; turn order enforced; pile-on limited.
    Critique,
    /// Wiki pin: a collaboratively edited post pinned above the OP; edits pass
    /// a lightweight approval queue.
    WikiPin,
    /// Collaborative fiction: posts stitch into a single narrative; a "compile"
    /// action renders the thread as one readable story.
    CollabFic,
    /// Prompt: posted by the prompt engine; replies are flash fiction;
    /// community votes decide favorites.
    Prompt,
    /// Character voice: posts render as a character from the poster's work.
    CharacterVoice,
}

impl ThreadMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Ama => "ama",
            Self::ReadingGroup => "reading_group",
            Self::Critique => "critique",
            Self::WikiPin => "wiki_pin",
            Self::CollabFic => "collab_fic",
            Self::Prompt => "prompt",
            Self::CharacterVoice => "character_voice",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "plain" => Some(Self::Plain),
            "ama" => Some(Self::Ama),
            "reading_group" => Some(Self::ReadingGroup),
            "critique" => Some(Self::Critique),
            "wiki_pin" => Some(Self::WikiPin),
            "collab_fic" => Some(Self::CollabFic),
            "prompt" => Some(Self::Prompt),
            "character_voice" => Some(Self::CharacterVoice),
            _ => None,
        }
    }

    /// Whether this mode restricts new posts to a turn queue (critique).
    pub fn uses_turn_queue(self) -> bool {
        matches!(self, Self::Critique)
    }

    /// Whether this mode has a schedule of sections (reading group).
    pub fn uses_schedule(self) -> bool {
        matches!(self, Self::ReadingGroup)
    }

    /// Whether this mode has a wiki pin.
    pub fn uses_wiki_pin(self) -> bool {
        matches!(self, Self::WikiPin)
    }
}

impl Default for ThreadMode {
    fn default() -> Self {
        Self::Plain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_modes() -> Vec<ThreadMode> {
        vec![
            ThreadMode::Plain,
            ThreadMode::Ama,
            ThreadMode::ReadingGroup,
            ThreadMode::Critique,
            ThreadMode::WikiPin,
            ThreadMode::CollabFic,
            ThreadMode::Prompt,
            ThreadMode::CharacterVoice,
        ]
    }

    #[test]
    fn every_mode_roundtrips() {
        for mode in all_modes() {
            let s = mode.as_str();
            let parsed = ThreadMode::from_str(s).unwrap_or_else(|| {
                panic!("ThreadMode::from_str({s:?}) returned None");
            });
            assert_eq!(mode, parsed, "roundtrip mismatch for {mode:?}");
        }
    }

    #[test]
    fn unknown_mode_returns_none() {
        assert_eq!(ThreadMode::from_str("telepathy"), None);
        assert_eq!(ThreadMode::from_str(""), None);
    }

    #[test]
    fn plain_is_default() {
        assert_eq!(ThreadMode::default(), ThreadMode::Plain);
    }

    #[test]
    fn turn_queue_is_critique_only() {
        for mode in all_modes() {
            let expected = matches!(mode, ThreadMode::Critique);
            assert_eq!(mode.uses_turn_queue(), expected, "{mode:?}");
        }
    }

    #[test]
    fn schedule_is_reading_group_only() {
        for mode in all_modes() {
            let expected = matches!(mode, ThreadMode::ReadingGroup);
            assert_eq!(mode.uses_schedule(), expected, "{mode:?}");
        }
    }
}
