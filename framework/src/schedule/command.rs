//! A console command on the schedule.
//!
//! [`Schedule::command`](super::Schedule::command) takes the command the
//! way it is typed behind the name of the console binary,
//! `"emails:send --force"`, and runs it in the scheduler's process. The
//! command line is read and checked when the schedule is built. A
//! scheduled task runs when nobody is watching, so a name with a typing
//! error has to stop the boot and not fail at three in the morning.

use crate::console::{self, CommandEntry};
use crate::error::FrameworkError;

/// A command line that was checked against the command it names.
pub(super) struct ScheduledCommand {
    /// The command line with one space between its words, for the name
    /// of the task.
    pub(super) line: String,
    /// The about text of the command, for the description of the task.
    pub(super) about: Option<String>,
    entry: &'static CommandEntry,
    matches: clap::ArgMatches,
}

impl ScheduledCommand {
    pub(super) fn parse(command: &str) -> Result<Self, FrameworkError> {
        let words = split(command)?;
        let Some(name) = words.first() else {
            return Err(FrameworkError::internal(
                "Schedule::command needs a command: the command line is empty",
            ));
        };
        let Some(entry) = console::find(name) else {
            let known: Vec<&str> = console::list().iter().map(|entry| entry.name).collect();
            return Err(FrameworkError::internal(format!(
                "Schedule::command(`{command}`): no console command is named `{name}`. \
                 Registered commands: {}",
                known.join(", ")
            )));
        };
        // The arguments are parsed here and the result is kept, so what
        // runs on the schedule is what was checked at boot.
        let matches = (entry.clap_builder)()
            .try_get_matches_from(&words)
            .map_err(|error| {
                FrameworkError::internal(format!(
                    "Schedule::command(`{command}`): the arguments do not parse for \
                     `{name}`: {}",
                    error.render().to_string().trim_end()
                ))
            })?;

        Ok(Self {
            line: words.join(" "),
            about: entry.about(),
            entry,
            matches,
        })
    }

    /// Run the command. The error it returns goes to the scheduler, which
    /// reports a failed task; the console is not asked to print it as
    /// well.
    pub(super) async fn run(&self) -> Result<(), FrameworkError> {
        (self.entry.handler)(&self.matches).await
    }
}

/// Split a command line into words the way a shell does, as far as a
/// command line in source code needs it.
///
/// Words are separated by whitespace. Single quotes keep everything
/// between them as it is. Double quotes do the same, and inside them a
/// backslash in front of `"` or `\` gives that character. Outside quotes
/// a backslash gives the character behind it. Nothing else a shell does
/// happens here: no variables, no globbing, no pipes. The line is run by
/// the console, not by a shell.
pub(super) fn split(line: &str) -> Result<Vec<String>, FrameworkError> {
    let mut words = Vec::new();
    let mut word = String::new();
    // A pair of quotes with nothing between them is a word, an empty one.
    let mut in_word = false;
    let mut chars = line.chars();

    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(inner) => word.push(inner),
                        None => return Err(unfinished(line, "a single quote is not closed")),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '\\')) => word.push(escaped),
                            Some(other) => {
                                word.push('\\');
                                word.push(other);
                            }
                            None => {
                                return Err(unfinished(line, "a double quote is not closed"));
                            }
                        },
                        Some(inner) => word.push(inner),
                        None => return Err(unfinished(line, "a double quote is not closed")),
                    }
                }
            }
            '\\' => match chars.next() {
                Some(escaped) => {
                    in_word = true;
                    word.push(escaped);
                }
                None => return Err(unfinished(line, "a backslash has nothing behind it")),
            },
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

fn unfinished(line: &str, what: &str) -> FrameworkError {
    FrameworkError::internal(format!("Schedule::command(`{line}`): {what}"))
}

#[cfg(test)]
mod tests {
    use super::split;

    fn words(line: &str) -> Vec<String> {
        split(line).unwrap_or_else(|e| panic!("`{line}` must split: {e}"))
    }

    #[test]
    fn whitespace_separates_words() {
        assert_eq!(words("emails:send --force"), ["emails:send", "--force"]);
        assert_eq!(words("  a \t b\n"), ["a", "b"]);
        assert!(words("").is_empty());
        assert!(words("   ").is_empty());
    }

    #[test]
    fn quotes_keep_a_word_together() {
        assert_eq!(
            words(r#"report --title "Weekly numbers" --to 'a b'"#),
            ["report", "--title", "Weekly numbers", "--to", "a b"]
        );
        assert_eq!(words(r#"--name="Ada Lovelace""#), ["--name=Ada Lovelace"]);
    }

    #[test]
    fn an_empty_pair_of_quotes_is_an_empty_word() {
        assert_eq!(words(r#"tag --value """#), ["tag", "--value", ""]);
        assert_eq!(words("tag ''"), ["tag", ""]);
    }

    #[test]
    fn a_backslash_gives_the_character_behind_it() {
        assert_eq!(words(r"a\ b"), ["a b"]);
        assert_eq!(words(r#""say \"hi\"""#), [r#"say "hi""#]);
        assert_eq!(words(r#""a\\b""#), [r"a\b"]);
        assert_eq!(
            words(r#""C:\temp""#),
            [r"C:\temp"],
            "inside double quotes a backslash escapes a quote or a backslash only"
        );
        assert_eq!(words(r"'a\b'"), [r"a\b"], "single quotes escape nothing");
    }

    #[test]
    fn a_line_that_does_not_end_is_refused() {
        for line in ["say 'hi", r#"say "hi"#, r"say hi\", r#""a\"#] {
            let error = split(line).expect_err("the line does not end");
            assert!(
                error.message().contains(line),
                "the error must show the line: {}",
                error.message()
            );
        }
    }
}
