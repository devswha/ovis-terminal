//! The work plan's numbered todo blocks and what each one declares.

use super::markdown::ids;

/// One `- [ ] N. Title` block.
#[derive(Debug, Clone, Default)]
pub(super) struct Todo {
    pub(super) number: u16,
    pub(super) landed: bool,
    pub(super) line: usize,
    pub(super) closes: Vec<String>,
    pub(super) delivers: Vec<String>,
    pub(super) excludes: Vec<String>,
    pub(super) wave: Option<u16>,
    pub(super) blocked_by: Vec<u16>,
    pub(super) blocks: Vec<u16>,
    pub(super) write_scope: Vec<String>,
    pub(super) scenarios: Vec<String>,
    pub(super) has_qa: bool,
}

/// Every numbered todo plus the numbers a scope decision removed.
#[derive(Debug, Clone, Default)]
pub(super) struct Plan {
    pub(super) todos: Vec<Todo>,
    pub(super) removed: Vec<u16>,
}

impl Plan {
    pub(super) fn todo(&self, number: u16) -> Option<&Todo> {
        self.todos.iter().find(|todo| todo.number == number)
    }

    /// Whether `number` is a todo block or a todo the plan records as removed.
    pub(super) fn accounts_for(&self, number: u16) -> bool {
        self.todo(number).is_some() || self.removed.contains(&number)
    }
}

/// `(landed, number)` of a `- [ ] N. ` / `- [x] N. ` heading line.
fn heading(line: &str) -> Option<(bool, u16)> {
    let rest = line.strip_prefix("- [")?;
    let (mark, rest) = rest.split_once("] ")?;
    let (number, _) = rest.split_once(". ")?;
    Some((mark == "x", number.parse().ok()?))
}

fn numbers(text: &str) -> Vec<u16> {
    text.split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect()
}

/// Write-scope paths: backticked segments, else comma-separated paths without notes.
fn scope_paths(text: &str) -> Vec<String> {
    let quoted: Vec<String> = text
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    if !quoted.is_empty() {
        return quoted;
    }
    text.split(", ")
        .map(|part| {
            part.split(" (")
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned()
        })
        .filter(|part| !part.is_empty() && !part.contains(' '))
        .collect()
}

fn parallelization(todo: &mut Todo, line: &str) {
    for field in line.split(" | ").map(str::trim) {
        if let Some(wave) = field.strip_prefix("Parallelization: Wave ") {
            todo.wave = wave.trim().parse().ok();
        } else if let Some(list) = field.strip_prefix("Blocked by: ") {
            todo.blocked_by = numbers(list);
        } else if let Some(list) = field.strip_prefix("Blocks: ") {
            todo.blocks = numbers(list);
        } else if let Some(scope) = field.strip_prefix("Write scope: ") {
            todo.write_scope = scope_paths(scope);
        }
    }
}

fn scenarios(block: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for part in block.split("--scenario ").skip(1) {
        let id: String = part
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_'))
            .collect();
        if id.contains('/') && id != "all/all" && !found.contains(&id) {
            found.push(id);
        }
    }
    found
}

fn finish(mut todo: Todo, block: &[&str]) -> Todo {
    let text = block.join("\n");
    for line in block.iter().map(|line| line.trim_start()) {
        for field in line.split(" | ") {
            if let Some(list) = field.strip_prefix("Closes:") {
                todo.closes = ids(list, "GAP");
            } else if let Some(list) = field.strip_prefix("Delivers:") {
                todo.delivers = ids(list, "IDEAL");
            }
        }
        if line.starts_with("Parallelization:") {
            parallelization(&mut todo, line);
        } else if line.starts_with("QA scenarios") {
            todo.has_qa = line.contains("happy") && line.contains("failure");
        }
    }
    todo.excludes = ids(&text, "EXCL");
    todo.excludes.dedup();
    todo.scenarios = scenarios(&text);
    todo
}

/// Numbers a scope decision lists as removed (`... todos 38 and 39 ... were removed`).
fn removed(text: &str) -> Vec<u16> {
    let mut found = Vec::new();
    for line in text.lines().filter(|line| line.contains("removed")) {
        for part in line.split("todos ").skip(1) {
            for word in part.split_whitespace() {
                if let Ok(number) = word.trim_end_matches(',').parse::<u16>() {
                    found.push(number);
                } else if word != "and" {
                    break;
                }
            }
        }
    }
    found.sort_unstable();
    found.dedup();
    found
}

/// Parses every numbered todo block of `text`.
pub(super) fn parse(text: &str) -> Plan {
    let mut todos = Vec::new();
    let mut open: Option<(Todo, Vec<&str>)> = None;
    for (index, line) in text.lines().enumerate() {
        let boundary = line.starts_with("- [") || line.starts_with('#');
        if boundary && let Some((todo, block)) = open.take() {
            todos.push(finish(todo, &block));
        }
        if let Some((landed, number)) = heading(line) {
            let todo = Todo {
                number,
                landed,
                line: index.saturating_add(1),
                ..Todo::default()
            };
            open = Some((todo, vec![line]));
        } else if let Some((_, block)) = open.as_mut() {
            block.push(line);
        }
    }
    if let Some((todo, block)) = open {
        todos.push(finish(todo, &block));
    }
    Plan {
        todos,
        removed: removed(text),
    }
}

#[cfg(test)]
mod tests;
