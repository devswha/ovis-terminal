//! Markdown table rows of the binding ledgers.

/// The trimmed cells of a `| a | b |` row, or `None` for any other line.
pub(super) fn cells(line: &str) -> Option<Vec<&str>> {
    let inner = line.trim().strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(str::trim).collect())
}

/// The lines of the section that starts with the heading `title` and ends at the next
/// heading of the same or a higher level.
pub(super) fn section<'text>(text: &'text str, title: &str) -> Option<Vec<&'text str>> {
    let mut lines = text.lines().skip_while(|line| line.trim_end() != title);
    let heading = lines.next()?;
    let level = heading.chars().take_while(|ch| *ch == '#').count();
    Some(
        lines
            .take_while(|line| {
                let depth = line.chars().take_while(|ch| *ch == '#').count();
                depth == 0 || depth > level || line.chars().nth(depth).is_none_or(|ch| ch != ' ')
            })
            .collect(),
    )
}

/// Every `<PREFIX>-<digits>` token in `text`, in order of appearance.
pub(super) fn ids(text: &str, prefix: &str) -> Vec<String> {
    let marker = format!("{prefix}-");
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(&marker) {
        let before = rest.get(..at).and_then(|head| head.chars().next_back());
        let tail = rest
            .get(at.saturating_add(marker.len())..)
            .unwrap_or_default();
        let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() && !before.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '-') {
            found.push(format!("{marker}{digits}"));
        }
        rest = tail;
    }
    found
}

/// The ids of a ledger table: the first cell of each `| <PREFIX>-NNN |` row.
pub(super) fn ledger_rows<'text>(text: &'text str, prefix: &str) -> Vec<(String, Vec<&'text str>)> {
    text.lines()
        .filter_map(cells)
        .filter_map(|row| {
            let id = row.first()?;
            let digits = id.strip_prefix(prefix)?.strip_prefix('-')?;
            let valid = !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
            valid.then(|| ((*id).to_owned(), row))
        })
        .collect()
}
