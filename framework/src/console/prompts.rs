//! The prompts a command shows beyond [`ask`](super::ask) and
//! [`confirm`](super::confirm): a default answer, a hidden answer, menus, a
//! progress bar and a form of several prompts.
//!
//! Each one reads one line, from a terminal or from a pipe alike, so a
//! command that asks works the same run by a person and run by a script
//! that pipes its answers in. Under [`test`](super::test) each one takes
//! the answer the test prepared, and its output is plain text.

use super::io::{self, Echo, Verbosity};
use crate::error::FrameworkError;
use std::collections::BTreeSet;
use std::io::{IsTerminal, Write};

/// Ask a question whose empty answer is `default`.
///
/// The hint behind the question shows the default: `Name? [Ada]`. A test
/// answers with an empty string to take it.
///
/// # Errors
///
/// Every error of [`ask`](super::ask).
pub fn ask_with_default(question: &str, default: &str) -> Result<String, FrameworkError> {
    let answer = io::read_answer(
        question,
        &format!("{question} [{default}] "),
        None,
        Echo::Shown,
    )?;
    if answer.trim().is_empty() {
        Ok(default.to_owned())
    } else {
        Ok(answer)
    }
}

/// Ask for a password, a token or another answer nobody should see.
///
/// On a terminal the answer is not echoed while it is typed. It is never
/// written anywhere, and a test does not capture it: the captured output
/// holds the question alone. From a pipe it reads one line, as every
/// prompt does. Laravel Prompts calls this `password`.
///
/// # Errors
///
/// Every error of [`ask`](super::ask), and when the standard input is a
/// terminal while neither output stream is one, since the echo can only be
/// turned off where the answer is typed.
pub fn secret(question: &str) -> Result<String, FrameworkError> {
    io::read_answer(question, &format!("{question} "), None, Echo::Hidden)
}

/// Ask the person to choose one of `options`, and return its index.
///
/// The answer is an option as it is written, or the number shown in front
/// of it. An empty answer is `default` when there is one. Laravel Prompts
/// calls this `select`, and `InteractsWithIO::choice` asks the same.
///
/// ```rust,no_run
/// # fn example() -> Result<(), suprnova::FrameworkError> {
/// let role = suprnova::console::select("Role?", &["Member", "Owner"], Some(0))?;
/// # Ok(()) }
/// ```
///
/// # Errors
///
/// When the answer is not one of `options`; the error names every option.
/// When `options` is empty or `default` is not one of its indexes, since
/// no answer could be right. Every error of [`ask`](super::ask).
pub fn select<L: AsRef<str>>(
    question: &str,
    options: &[L],
    default: Option<usize>,
) -> Result<usize, FrameworkError> {
    let labels = labels(question, options)?;
    check_indexes(question, &labels, default.iter().copied())?;
    let numbered: Vec<(String, &str)> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| (index.to_string(), label.as_str()))
        .collect();
    let hint = default.map(|index| labels[index].as_str());
    let answer = io::read_answer(
        question,
        &menu_prompt(question, &numbered, hint),
        Some(&labels),
        Echo::Shown,
    )?;
    let answer = answer.trim();
    match default {
        Some(index) if answer.is_empty() => Ok(index),
        _ => pick(question, &labels, answer, true),
    }
}

/// Ask the person to choose one of `options`, each a key and the label it
/// is shown with, and return the chosen key.
///
/// The answer is a label or a key. An empty answer is the option whose key
/// is `default`, when there is one. Laravel Prompts' `select` returns the
/// key of an associative array of options the same way.
///
/// ```rust,no_run
/// # fn example() -> Result<(), suprnova::FrameworkError> {
/// let plan = suprnova::console::select_keyed(
///     "Plan?",
///     &[("basic", "Basic"), ("pro", "Professional")],
///     None,
/// )?;
/// # Ok(()) }
/// ```
///
/// # Errors
///
/// As [`select`], and when `default` names no key of `options`.
pub fn select_keyed<K, L>(
    question: &str,
    options: &[(K, L)],
    default: Option<&str>,
) -> Result<K, FrameworkError>
where
    K: AsRef<str> + Clone,
    L: AsRef<str>,
{
    let labels = labels(
        question,
        &options
            .iter()
            .map(|(_, label)| label.as_ref())
            .collect::<Vec<_>>(),
    )?;
    let keys: Vec<&str> = options.iter().map(|(key, _)| key.as_ref()).collect();
    let default = match default {
        None => None,
        Some(key) => Some(keys.iter().position(|k| *k == key).ok_or_else(|| {
            FrameworkError::internal(format!(
                "`{question}`: the default `{key}` is not a key of its options"
            ))
        })?),
    };
    let keyed: Vec<(String, &str)> = keys
        .iter()
        .zip(&labels)
        .map(|(key, label)| ((*key).to_owned(), label.as_str()))
        .collect();
    let hint = default.map(|index| labels[index].as_str());
    let answer = io::read_answer(
        question,
        &menu_prompt(question, &keyed, hint),
        Some(&labels),
        Echo::Shown,
    )?;
    let answer = answer.trim();
    let index = match default {
        Some(index) if answer.is_empty() => index,
        _ => match keys.iter().position(|key| *key == answer) {
            Some(index) if !labels.iter().any(|label| label == answer) => index,
            _ => pick(question, &labels, answer, false)?,
        },
    };
    Ok(options[index].0.clone())
}

/// Ask the person to choose any number of `options`, and return the
/// chosen indexes in the order of the options.
///
/// The answer is the chosen options separated by commas, each written as
/// it is shown or as its number: `Member,Owner`. An empty answer is
/// `defaults`. Laravel Prompts calls this `multiselect`.
///
/// # Errors
///
/// When one of the answers is not one of `options`; the error names every
/// option. When `options` is empty or a default is not one of its indexes.
/// Every error of [`ask`](super::ask).
pub fn multiselect<L: AsRef<str>>(
    question: &str,
    options: &[L],
    defaults: &[usize],
) -> Result<Vec<usize>, FrameworkError> {
    let labels = labels(question, options)?;
    check_indexes(question, &labels, defaults.iter().copied())?;
    let numbered: Vec<(String, &str)> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| (index.to_string(), label.as_str()))
        .collect();
    let hint = defaults
        .iter()
        .map(|index| labels[*index].as_str())
        .collect::<Vec<_>>()
        .join(",");
    let answer = io::read_answer(
        question,
        &menu_prompt(
            question,
            &numbered,
            (!hint.is_empty()).then_some(hint.as_str()),
        ),
        Some(&labels),
        Echo::Shown,
    )?;
    let chosen: BTreeSet<usize> = if answer.trim().is_empty() {
        defaults.iter().copied().collect()
    } else {
        answer
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| pick(question, &labels, part, true))
            .collect::<Result<_, _>>()?
    };
    Ok(chosen.into_iter().collect())
}

/// The labels of `options`, refused when there are none to choose from.
fn labels<L: AsRef<str>>(question: &str, options: &[L]) -> Result<Vec<String>, FrameworkError> {
    if options.is_empty() {
        return Err(FrameworkError::internal(format!(
            "`{question}` offers no options to choose from"
        )));
    }
    Ok(options.iter().map(|o| o.as_ref().to_owned()).collect())
}

/// Refuse a default that is not an index of `labels`.
fn check_indexes(
    question: &str,
    labels: &[String],
    indexes: impl IntoIterator<Item = usize>,
) -> Result<(), FrameworkError> {
    for index in indexes {
        if index >= labels.len() {
            return Err(FrameworkError::internal(format!(
                "`{question}`: the default {index} is not an index of its {} options",
                labels.len()
            )));
        }
    }
    Ok(())
}

/// The index of the option `answer` names: its label, or, with `numbers`,
/// the number it is shown with. A label wins over a number, so an option
/// written `1` is always that option.
fn pick(
    question: &str,
    labels: &[String],
    answer: &str,
    numbers: bool,
) -> Result<usize, FrameworkError> {
    if let Some(index) = labels.iter().position(|label| label == answer) {
        return Ok(index);
    }
    if numbers
        && let Ok(index) = answer.parse::<usize>()
        && index < labels.len()
    {
        return Ok(index);
    }
    let what = if answer.is_empty() {
        format!("`{question}` needs an answer")
    } else {
        format!("`{answer}` is not an option of `{question}`")
    };
    Err(FrameworkError::bad_request(format!(
        "{what}: answer one of {}",
        io::quoted_list(labels)
    )))
}

/// The question, one line per option with the key it is chosen by, and
/// the default in brackets.
fn menu_prompt(question: &str, options: &[(String, &str)], default: Option<&str>) -> String {
    let mut prompt = format!("{question}\n");
    for (key, label) in options {
        prompt.push_str(&format!("  [{key}] {label}\n"));
    }
    match default {
        Some(default) => prompt.push_str(&format!("[{default}] ")),
        None => prompt.push_str("> "),
    }
    prompt
}

/// A progress bar for work of a known number of steps.
///
/// On a terminal it redraws one line as the work advances. Where the
/// output is not a terminal, a pipe, a log or a test, it writes one plain
/// line when it finishes, `Sync 3/3`, so the output holds the result and
/// not one line per step. It writes nothing in a quiet run. Laravel Prompts
/// calls this `progress`.
///
/// ```rust,no_run
/// use suprnova::console::Progress;
///
/// let mut bar = Progress::new("Importing", 2);
/// bar.advance(1);
/// bar.hint("users.csv");
/// bar.advance(1);
/// bar.finish();
/// ```
#[derive(Debug)]
pub struct Progress {
    label: String,
    hint: Option<String>,
    total: u64,
    done: u64,
    /// Whether the bar redraws a terminal line, decided when it starts.
    redraws: bool,
    finished: bool,
}

/// The width of the bar between its brackets, in characters.
const BAR_WIDTH: u64 = 28;

impl Progress {
    /// A bar at 0 of `total` steps, labelled `label`.
    pub fn new(label: impl Into<String>, total: u64) -> Self {
        let redraws = !io::is_captured()
            && io::verbosity() != Verbosity::Quiet
            && std::io::stdout().is_terminal();
        let bar = Self {
            label: label.into(),
            hint: None,
            total,
            done: 0,
            redraws,
            finished: false,
        };
        bar.draw();
        bar
    }

    /// Count `steps` more steps as done. The count stops at the total.
    pub fn advance(&mut self, steps: u64) -> &mut Self {
        self.done = self.done.saturating_add(steps).min(self.total);
        self.draw();
        self
    }

    /// Show `label` in front of the bar from now on.
    pub fn label(&mut self, label: impl Into<String>) -> &mut Self {
        self.label = label.into();
        self.draw();
        self
    }

    /// Show `hint` behind the count, such as the item being worked on.
    pub fn hint(&mut self, hint: impl Into<String>) -> &mut Self {
        self.hint = Some(hint.into());
        self.draw();
        self
    }

    /// The steps done so far.
    pub fn done(&self) -> u64 {
        self.done
    }

    /// The steps the work has.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// End the bar: on a terminal its line is ended, and elsewhere its one
    /// plain line is written.
    pub fn finish(mut self) {
        self.finished = true;
        if self.redraws {
            self.draw();
            io::write_output("\n");
        } else {
            io::line(self.text());
        }
    }

    /// `<label> <done>/<total>`, and the hint in brackets behind it.
    fn text(&self) -> String {
        match &self.hint {
            Some(hint) => format!("{} {}/{} ({hint})", self.label, self.done, self.total),
            None => format!("{} {}/{}", self.label, self.done, self.total),
        }
    }

    /// Redraw the terminal line. Elsewhere nothing is drawn until the end.
    fn draw(&self) {
        if !self.redraws {
            return;
        }
        let filled = BAR_WIDTH * self.done / self.total.max(1);
        let filled = if self.total == 0 { BAR_WIDTH } else { filled };
        let bar = format!(
            "{}{}",
            "#".repeat(filled as usize),
            "-".repeat((BAR_WIDTH - filled) as usize)
        );
        let hint = self
            .hint
            .as_deref()
            .map(|hint| format!(" ({hint})"))
            .unwrap_or_default();
        // `\r` back to the start of the line and `ESC [2K` to clear it, so a
        // shorter label leaves nothing of the longer one behind.
        let mut out = std::io::stdout().lock();
        let _ = write!(
            out,
            "\r\x1b[2K{} [{bar}] {}/{}{hint}",
            self.label, self.done, self.total
        )
        .and_then(|()| out.flush());
    }
}

impl Drop for Progress {
    /// A bar dropped before it finished, by an early return or an error,
    /// ends its terminal line, so what is printed next starts on a line
    /// of its own.
    fn drop(&mut self) {
        if self.redraws && !self.finished {
            io::write_output("\n");
        }
    }
}

/// Map every item of `items` through `f` behind a progress bar labelled
/// `label`, and return the results in order.
///
/// `f` gets each item and the bar, to set a [`hint`](Progress::hint) or a
/// [`label`](Progress::label); the bar advances one step after each item.
/// Laravel Prompts' `progress` takes its steps and callback the same way.
///
/// ```rust,no_run
/// let sizes = suprnova::console::progress("Sync", ["a.txt", "b.txt"], |name, bar| {
///     bar.hint(name);
///     name.len()
/// });
/// ```
pub fn progress<I, T, R, F>(label: &str, items: I, mut f: F) -> Vec<R>
where
    I: IntoIterator<Item = T>,
    F: FnMut(T, &mut Progress) -> R,
{
    let items: Vec<T> = items.into_iter().collect();
    let mut bar = Progress::new(label, items.len() as u64);
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        results.push(f(item, &mut bar));
        bar.advance(1);
    }
    bar.finish();
    results
}

/// Start a form: several prompts asked in order, whose answers come back
/// together under the names you give them. Laravel Prompts calls this
/// `form`.
///
/// ```rust,no_run
/// # fn example() -> Result<(), suprnova::FrameworkError> {
/// let answers = suprnova::console::form()
///     .text("name", "Name?")
///     .secret("token", "API token?")
///     .confirm("admin", "Make them an admin?", false)
///     .select("plan", "Plan?", &["Basic", "Professional"], Some(0))
///     .submit()?;
/// let name = answers.text("name").unwrap_or_default();
/// # Ok(()) }
/// ```
pub fn form() -> Form {
    Form { steps: Vec::new() }
}

/// The prompts of a form, asked by [`submit`](Self::submit). Built by
/// [`form()`].
#[must_use = "a form asks nothing until `.submit()`"]
pub struct Form {
    steps: Vec<(String, Step)>,
}

enum Step {
    Text(String),
    Secret(String),
    Confirm(String, bool),
    Select(String, Vec<String>, Option<usize>),
}

impl Form {
    /// Ask `question` and keep the answer under `name`, as
    /// [`ask`](super::ask) does.
    pub fn text(mut self, name: impl Into<String>, question: impl Into<String>) -> Self {
        self.steps.push((name.into(), Step::Text(question.into())));
        self
    }

    /// Ask `question` with a hidden answer and keep it under `name`, as
    /// [`secret`] does.
    pub fn secret(mut self, name: impl Into<String>, question: impl Into<String>) -> Self {
        self.steps
            .push((name.into(), Step::Secret(question.into())));
        self
    }

    /// Ask the yes-or-no `question` and keep the answer under `name`, as
    /// [`confirm`](super::confirm) does.
    pub fn confirm(
        mut self,
        name: impl Into<String>,
        question: impl Into<String>,
        default: bool,
    ) -> Self {
        self.steps
            .push((name.into(), Step::Confirm(question.into(), default)));
        self
    }

    /// Ask the person to choose one of `options` and keep the index under
    /// `name`, as [`select`] does.
    pub fn select<L: AsRef<str>>(
        mut self,
        name: impl Into<String>,
        question: impl Into<String>,
        options: &[L],
        default: Option<usize>,
    ) -> Self {
        let options = options.iter().map(|o| o.as_ref().to_owned()).collect();
        self.steps
            .push((name.into(), Step::Select(question.into(), options, default)));
        self
    }

    /// Ask every prompt in the order it was added, and return the answers.
    ///
    /// # Errors
    ///
    /// When two prompts share a name, before anything is asked: one answer
    /// would replace the other. Otherwise the first error of a prompt, and
    /// the prompts after it are not asked.
    pub fn submit(self) -> Result<FormAnswers, FrameworkError> {
        let mut names = BTreeSet::new();
        for (name, _) in &self.steps {
            if !names.insert(name.as_str()) {
                return Err(FrameworkError::internal(format!(
                    "the form names two prompts `{name}`; give each prompt its own name"
                )));
            }
        }
        let mut values = Vec::with_capacity(self.steps.len());
        for (name, step) in self.steps {
            let value = match step {
                Step::Text(question) => FormValue::Text(super::ask(&question)?),
                Step::Secret(question) => FormValue::Secret(secret(&question)?),
                Step::Confirm(question, default) => {
                    FormValue::Bool(super::confirm(&question, default)?)
                }
                Step::Select(question, options, default) => {
                    FormValue::Index(select(&question, &options, default)?)
                }
            };
            values.push((name, value));
        }
        Ok(FormAnswers { values })
    }
}

/// One answer of a form.
#[derive(Clone, PartialEq, Eq)]
pub enum FormValue {
    /// The answer to a [`Form::text`] prompt.
    Text(String),
    /// The answer to a [`Form::secret`] prompt. Its `Debug` output hides
    /// it, so a log of the answers does not show it.
    Secret(String),
    /// The answer to a [`Form::confirm`] prompt.
    Bool(bool),
    /// The index a [`Form::select`] prompt answered.
    Index(usize),
}

impl std::fmt::Debug for FormValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::Secret(_) => f.debug_tuple("Secret").field(&"[REDACTED]").finish(),
            Self::Bool(value) => f.debug_tuple("Bool").field(value).finish(),
            Self::Index(index) => f.debug_tuple("Index").field(index).finish(),
        }
    }
}

/// The answers of a submitted [`Form`], by the names of its prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormAnswers {
    values: Vec<(String, FormValue)>,
}

impl FormAnswers {
    /// The answer under `name`, of any kind.
    pub fn get(&self, name: &str) -> Option<&FormValue> {
        self.values
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    /// The text a [`Form::text`] or [`Form::secret`] prompt under `name`
    /// answered.
    pub fn text(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            FormValue::Text(text) | FormValue::Secret(text) => Some(text),
            _ => None,
        }
    }

    /// The answer a [`Form::confirm`] prompt under `name` gave.
    pub fn confirmed(&self, name: &str) -> Option<bool> {
        match self.get(name)? {
            FormValue::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The index a [`Form::select`] prompt under `name` answered.
    pub fn selected(&self, name: &str) -> Option<usize> {
        match self.get(name)? {
            FormValue::Index(index) => Some(*index),
            _ => None,
        }
    }
}
