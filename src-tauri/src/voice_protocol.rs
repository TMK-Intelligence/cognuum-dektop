//! Pure activation policy. Partial hypotheses can light the UI, never execute.
//! Only an anchored wake phrase in the final hypothesis authorizes a command.
pub const MAX_COMMAND_CHARS: usize = 500;

pub fn after_wake(text: &str, short_wake: bool) -> Option<String> {
    let words: Vec<_> = text.split_whitespace().collect();
    let clean = |s: &str| {
        s.trim_matches(|c: char| !c.is_alphanumeric())
            .to_ascii_lowercase()
    };
    // Whole-token phonetic ASR spellings, never fuzzy substring matching.
    let skip = if words
        .first()
        .is_some_and(|word| ["heymax", "haymax", "haemaks"].contains(&clean(word).as_str()))
    {
        1
    } else if words.len() >= 2
        && ["hey", "hay"].contains(&clean(words[0]).as_str())
        && ["max", "macs", "maks"].contains(&clean(words[1]).as_str())
    {
        2
    } else if short_wake && words.first().is_some_and(|word| clean(word) == "max") {
        1
    } else {
        return None;
    };
    Some(words[skip..].join(" "))
}

pub fn command(text: &str) -> Option<String> {
    let text = text.trim();
    if text.chars().count() < 2
        || text.chars().count() > MAX_COMMAND_CHARS
        || text.chars().any(char::is_control)
        || ["cancel", "never mind", "nevermind", "stop listening"]
            .contains(&text.to_ascii_lowercase().as_str())
    {
        None
    } else {
        Some(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_breath_and_punctuation_keep_the_command() {
        assert_eq!(
            after_wake("HEY MAX, show Apple", false),
            Some("show Apple".into())
        );
        assert_eq!(
            after_wake("Max, go to settings", true),
            Some("go to settings".into())
        );
        assert_eq!(after_wake("hey max", true), Some(String::new()));
        assert_eq!(after_wake("max", true), Some(String::new()));
        assert_eq!(
            after_wake("hay macs open settings", false),
            Some("open settings".into())
        );
        assert_eq!(
            after_wake("HAEMAKS OPEN SETTINGS", false),
            Some("OPEN SETTINGS".into())
        );
    }
    #[test]
    fn conversation_and_partial_words_do_not_activate() {
        for text in [
            "I said hey max",
            "listenership",
            "hey maximum",
            "hey",
            "maximum",
            "maxwell",
            "haymarket",
            "haymaxwell",
            "the haemaks chart",
        ] {
            assert_eq!(after_wake(text, true), None);
        }
        assert_eq!(after_wake("listen show Apple", false), None);
        assert_eq!(after_wake("listen show Apple", true), None);
        assert_eq!(after_wake("max show Apple", false), None);
    }
    #[test]
    fn noise_cancel_and_oversized_commands_do_not_execute() {
        for text in [
            "",
            "x",
            "Cancel",
            "never mind",
            "stop listening",
            "load\nApple",
        ] {
            assert_eq!(command(text), None);
        }
        assert_eq!(command(&"a".repeat(501)), None);
        assert_eq!(command(" load Apple "), Some("load Apple".into()));
    }
}
