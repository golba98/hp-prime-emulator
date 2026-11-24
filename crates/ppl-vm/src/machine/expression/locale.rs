//! Runtime expression punctuation follows HSeparator, without rewriting strings,
//! comments, member names or canonical numeric storage.
use crate::error::{ErrorKind, RtResult, RuntimeError};
pub(super) fn expressions(text: &str, comma: bool) -> RtResult<Vec<String>> {
    let characters: Vec<_> = text.chars().collect();
    let mut result = Vec::new();
    let mut part = String::new();
    let mut index = 0;
    let mut quoted = false;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut depth = 0usize;
    while index < characters.len() {
        let ch = characters[index];
        let next = characters.get(index + 1).copied();
        if line_comment {
            part.push(ch);
            if ch == '\n' {
                line_comment = false;
            }
            index += 1;
            continue;
        }
        if block_comment {
            part.push(ch);
            if ch == '*' && next == Some('/') {
                part.push('/');
                index += 1;
                block_comment = false;
            }
            index += 1;
            continue;
        }
        if quoted {
            part.push(ch);
            if ch == '\\' {
                if let Some(next) = next {
                    part.push(next);
                    index += 1;
                }
            } else if ch == '"' {
                quoted = false;
            }
            index += 1;
            continue;
        }
        if ch == '"' {
            quoted = true;
            part.push(ch);
            index += 1;
            continue;
        }
        if ch == '/' && matches!(next, Some('/' | '*')) {
            line_comment = next == Some('/');
            block_comment = !line_comment;
            part.push(ch);
            part.push(next.unwrap());
            index += 2;
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => {
                if !part.trim().is_empty() {
                    result.push(std::mem::take(&mut part));
                }
                index += 1;
                continue;
            }
            ';' if comma => {
                part.push(',');
                index += 1;
                continue;
            }
            ',' if comma => {
                if !next.is_some_and(|c| c.is_ascii_digit()) {
                    return Err(RuntimeError::new(
                        ErrorKind::InvalidInput,
                        "A decimal comma must precede a digit; use semicolons between arguments",
                    ));
                }
                part.push('.');
                index += 1;
                continue;
            }
            '.' if comma
                && (next.is_some_and(|c| c.is_ascii_digit())
                    || (index > 0
                        && characters[index - 1].is_ascii_digit()
                        && !matches!(next, Some('*' | '/' | '^')))) =>
            {
                return Err(RuntimeError::new(
                    ErrorKind::InvalidInput,
                    "The selected number format uses a decimal comma",
                ));
            }
            _ => (),
        }
        part.push(ch);
        index += 1;
    }
    if !part.trim().is_empty() {
        result.push(part);
    }
    if result.is_empty() {
        return Err(RuntimeError::new(
            ErrorKind::InvalidInput,
            "Expression is empty",
        ));
    }
    Ok(result)
}
