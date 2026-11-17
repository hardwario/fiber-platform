//! Changing the components in `platform.yaml` line by line, so that everything else in the file
//! stays as the person wrote it, comments too.
//!
//! Only the block under the top-level `components:` key is touched: a line `  <id>: true` gets
//! its value changed, or a new one is added after the last of them. The caller reads the new
//! text back before it writes it ([`super::Platform::parse`]), so a file this module does not
//! understand is never written wrong, only refused.

use std::fmt;

/// Why the components could not be changed line by line.
#[derive(Debug, PartialEq)]
pub enum EditError {
    /// The components are not one per line (`components: {base: true}`, or something more
    /// elaborate). The message says how to fix that.
    NotOnePerLine { line: usize },
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            EditError::NotOnePerLine { line } => write!(
                f,
                "line {line}: the components can only be changed when they are written one per line, \
                 as in `  base: true`"
            ),
        }
    }
}

/// A line of the file, without its line ending.
struct Line<'a> {
    text: &'a str,
}

impl Line<'_> {
    fn is_blank_or_comment(&self) -> bool {
        let t = self.text.trim_start();
        t.is_empty() || t.starts_with('#')
    }

    fn is_indented(&self) -> bool {
        self.text.starts_with([' ', '\t'])
    }
}

/// Where the components are in the file.
struct Layout {
    /// The index of the `components:` line, and what follows the colon on it.
    header: Option<(usize, Header)>,
    entries: Vec<Entry>,
}

#[derive(Debug, PartialEq)]
enum Header {
    /// `components:` with the entries on the lines below.
    Block,
    /// `components: {}`, `components: ~` or `components: null`.
    Empty,
    /// Anything else after the colon.
    Flow,
}

/// One `  <id>: <value>` line.
struct Entry {
    index: usize,
    indent: String,
    id: String,
    /// The byte range of the value within the line.
    value: (usize, usize),
}

/// Splits a line into its key and what follows the colon, without a trailing comment.
/// `None` when the line is not `key: ...`.
fn key_and_value(text: &str) -> Option<(String, &str, usize)> {
    let start = text.len() - text.trim_start().len();
    let rest = &text[start..];
    let (key, after) = if let Some(quoted) = rest.strip_prefix('"').or_else(|| rest.strip_prefix('\'')) {
        let quote = rest.chars().next()?;
        let end = quoted.find(quote)?;
        (quoted[..end].to_owned(), &quoted[end + 1..])
    } else {
        let end = rest.find(':')?;
        (rest[..end].trim_end().to_owned(), &rest[end..])
    };
    let after = after.trim_start().strip_prefix(':')?;
    // A YAML key is followed by a colon and then a space or the end of the line.
    if !(after.is_empty() || after.starts_with([' ', '\t'])) {
        return None;
    }
    let value_start = text.len() - after.len() + (after.len() - after.trim_start().len());
    let value = without_comment(&text[value_start..]);
    Some((key, value, value_start))
}

/// The text before a ` #` comment, without trailing blanks.
fn without_comment(text: &str) -> &str {
    let mut end = text.len();
    let bytes = text.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            end = i;
            break;
        }
    }
    text[..end].trim_end()
}

fn layout(lines: &[Line]) -> Result<Layout, EditError> {
    let mut header = None;
    for (index, line) in lines.iter().enumerate() {
        if line.is_indented() || line.is_blank_or_comment() {
            continue;
        }
        if let Some((key, value, _)) = key_and_value(line.text)
            && key == "components"
        {
            let kind = match value {
                "" => Header::Block,
                "{}" | "~" | "null" => Header::Empty,
                _ => Header::Flow,
            };
            header = Some((index, kind));
            break;
        }
    }
    let mut entries = Vec::new();
    if let Some((start, Header::Block)) = header {
        for (index, line) in lines.iter().enumerate().skip(start + 1) {
            if line.is_blank_or_comment() {
                continue;
            }
            if !line.is_indented() {
                break;
            }
            let Some((id, value, value_start)) = key_and_value(line.text) else {
                return Err(EditError::NotOnePerLine { line: index + 1 });
            };
            if value != "true" && value != "false" {
                return Err(EditError::NotOnePerLine { line: index + 1 });
            }
            let indent = line.text[..line.text.len() - line.text.trim_start().len()].to_owned();
            entries.push(Entry {
                index,
                indent,
                id,
                value: (value_start, value_start + value.len()),
            });
        }
    }
    Ok(Layout { header, entries })
}

/// Splits the text into lines, each with its line ending.
fn split(text: &str) -> Vec<(&str, &str)> {
    text.split_inclusive('\n')
        .map(|l| {
            let body = l.trim_end_matches(['\n', '\r']);
            (body, &l[body.len()..])
        })
        .collect()
}

/// The line (from 1) on which the file sets `id`, if it is written one per line.
pub fn line_of(text: &str, id: &str) -> Option<usize> {
    let parts = split(text);
    let lines: Vec<Line> = parts.iter().map(|(text, _)| Line { text }).collect();
    let layout = layout(&lines).ok()?;
    layout.entries.iter().find(|e| e.id == id).map(|e| e.index + 1)
}

/// `text` with each `(id, on)` of `changes` set under `components:`.
pub fn set(text: &str, changes: &[(String, bool)]) -> Result<String, EditError> {
    let parts = split(text);
    let lines: Vec<Line> = parts.iter().map(|(text, _)| Line { text }).collect();
    let layout = layout(&lines)?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };

    let mut out: Vec<String> = parts.iter().map(|(body, end)| format!("{body}{end}")).collect();
    if let Some(last) = out.last_mut()
        && !last.ends_with('\n')
    {
        last.push_str(newline);
    }

    // Where new lines go, and how far they are indented.
    let (insert_at, indent) = match &layout.header {
        Some((index, Header::Flow)) => return Err(EditError::NotOnePerLine { line: index + 1 }),
        Some((index, Header::Empty)) => {
            let (body, _) = parts[*index];
            let (_, value, start) = key_and_value(body).unwrap_or_default();
            out[*index] = format!(
                "{}{}{newline}",
                body[..start].trim_end(),
                &body[start + value.len()..]
            );
            (index + 1, "  ".to_owned())
        }
        Some((index, Header::Block)) => match layout.entries.last() {
            Some(last) => (last.index + 1, layout.entries[0].indent.clone()),
            None => (index + 1, "  ".to_owned()),
        },
        None => {
            // No `components:` yet: before `settings:`, or at the end.
            let settings = lines.iter().position(|l| {
                !l.is_indented() && key_and_value(l.text).is_some_and(|(key, _, _)| key == "settings")
            });
            let at = settings.unwrap_or(out.len());
            out.insert(at, format!("components:{newline}"));
            (at + 1, "  ".to_owned())
        }
    };

    let mut added = Vec::new();
    for (id, on) in changes {
        let value = if *on { "true" } else { "false" };
        match layout.entries.iter().find(|e| &e.id == id) {
            Some(entry) => {
                let (body, end) = parts[entry.index];
                let (from, to) = entry.value;
                out[entry.index] = format!("{}{value}{}{end}", &body[..from], &body[to..]);
            }
            None => added.push(format!("{indent}{id}: {value}{newline}")),
        }
    }
    for (n, line) in added.into_iter().enumerate() {
        out.insert(insert_at + n, line);
    }
    Ok(out.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set1(text: &str, id: &str, on: bool) -> String {
        set(text, &[(id.to_owned(), on)]).unwrap()
    }

    #[test]
    fn a_value_changes_and_its_comment_stays() {
        let text = "# mine\nschema: 1\ncomponents:\n  base: true   # the board\n  node-red: false # later\n";
        assert_eq!(
            set1(text, "node-red", true),
            "# mine\nschema: 1\ncomponents:\n  base: true   # the board\n  node-red: true # later\n"
        );
    }

    #[test]
    fn a_new_component_goes_after_the_last_one() {
        let text =
            "schema: 1\ncomponents:\n    base: true\n# about settings\nsettings:\n  mqtt:\n    lan: true\n";
        assert_eq!(
            set1(text, "agent", true),
            "schema: 1\ncomponents:\n    base: true\n    agent: true\n# about settings\nsettings:\n  mqtt:\n    lan: true\n"
        );
    }

    #[test]
    fn comments_between_the_components_stay() {
        let text =
            "schema: 1\ncomponents:\n  # the board\n  base: true\n\n  # the broker\n  mosquitto: false\n";
        assert_eq!(
            set1(text, "mosquitto", true),
            "schema: 1\ncomponents:\n  # the board\n  base: true\n\n  # the broker\n  mosquitto: true\n"
        );
    }

    #[test]
    fn several_at_once() {
        let text = "schema: 1\ncomponents:\n  base: false\n";
        let changes = [
            ("mosquitto".to_owned(), true),
            ("base".to_owned(), true),
            ("agent".to_owned(), true),
        ];
        assert_eq!(
            set(text, &changes).unwrap(),
            "schema: 1\ncomponents:\n  base: true\n  mosquitto: true\n  agent: true\n"
        );
    }

    #[test]
    fn no_components_yet() {
        assert_eq!(
            set1("schema: 1\n", "base", true),
            "schema: 1\ncomponents:\n  base: true\n"
        );
        assert_eq!(
            set1("schema: 1", "base", true),
            "schema: 1\ncomponents:\n  base: true\n"
        );
        assert_eq!(
            set1("schema: 1\nsettings:\n  mqtt: {lan: true}\n", "base", true),
            "schema: 1\ncomponents:\n  base: true\nsettings:\n  mqtt: {lan: true}\n"
        );
        assert_eq!(
            set1("schema: 1\ncomponents:\n", "base", true),
            "schema: 1\ncomponents:\n  base: true\n"
        );
        assert_eq!(
            set1("schema: 1\ncomponents: {}  # none yet\n", "base", true),
            "schema: 1\ncomponents:  # none yet\n  base: true\n"
        );
        assert_eq!(
            set1("schema: 1\ncomponents: ~\n", "base", true),
            "schema: 1\ncomponents:\n  base: true\n"
        );
    }

    #[test]
    fn quoted_names() {
        let text = "schema: 1\ncomponents:\n  \"node-red\": false\n  'base': true\n";
        assert_eq!(
            set1(text, "node-red", true),
            "schema: 1\ncomponents:\n  \"node-red\": true\n  'base': true\n"
        );
        assert_eq!(line_of(text, "base"), Some(4));
    }

    #[test]
    fn windows_line_endings_stay() {
        let text = "schema: 1\r\ncomponents:\r\n  base: false\r\n";
        assert_eq!(
            set1(text, "base", true),
            "schema: 1\r\ncomponents:\r\n  base: true\r\n"
        );
        assert_eq!(
            set1(text, "agent", true),
            "schema: 1\r\ncomponents:\r\n  base: false\r\n  agent: true\r\n"
        );
    }

    #[test]
    fn not_one_per_line() {
        assert_eq!(
            set(
                "schema: 1\ncomponents: {base: true}\n",
                &[("agent".to_owned(), true)]
            ),
            Err(EditError::NotOnePerLine { line: 2 })
        );
        assert_eq!(
            set(
                "schema: 1\ncomponents:\n  base: true\n  ? agent\n  : true\n",
                &[("base".to_owned(), true)]
            ),
            Err(EditError::NotOnePerLine { line: 4 })
        );
    }

    #[test]
    fn lines_of_the_components() {
        let text = "schema: 1\ncomponents:\n  base: true\n  agent: true\nsettings:\n  base: x\n";
        assert_eq!(line_of(text, "base"), Some(3));
        assert_eq!(line_of(text, "agent"), Some(4));
        assert_eq!(line_of(text, "node-red"), None);
        assert_eq!(line_of("schema: 1\ncomponents: {base: true}\n", "base"), None);
    }

    #[test]
    fn a_comment_needs_a_blank_before_it() {
        assert_eq!(without_comment("true # x"), "true");
        assert_eq!(without_comment("a#b"), "a#b");
        assert_eq!(without_comment("# x"), "");
    }
}
