//! Compact binary UTF16 trees stored in G1 language resources.
use super::{Error, Result, short};

#[derive(Debug)]
enum Node {
    Unit(u16),
    Branch(Box<Node>, Box<Node>),
}

#[cfg(test)]
mod tests {
    use super::Decoder;

    #[test]
    fn interface_strings_discard_cr_and_skip_the_original_padding_byte() {
        let decoder = Decoder::new(&[3, 0, 0, 13, 0]).unwrap();
        // Two CR leaves, then NUL. The arbitrary next byte is padding, as in
        // the original reader; it must not become another interface string.
        assert_eq!(
            decoder.strings(&[3, 42], 1, &mut 100).unwrap(),
            vec![Vec::<u16>::new()]
        );
        assert_eq!(decoder.spans(&[3], &mut 100).unwrap(), vec![vec![13, 13]]);
        assert!(decoder.strings(&[3, 42], 2, &mut 100).is_err());
    }
}

#[derive(Debug)]
pub(super) struct Decoder(Node);
impl Decoder {
    pub fn new(data: &[u8]) -> Result<Self> {
        fn node(data: &[u8], at: usize, depth: usize) -> Result<(Node, usize)> {
            if depth > 64 {
                return Err(Error::new("Help dictionary exceeds its depth limit"));
            }
            let kind = *data
                .get(at)
                .ok_or_else(|| Error::new("Truncated Help dictionary"))?;
            let value = short(data, at + 1)?;
            let next = at + 3;
            let (left, right, end) = match kind {
                0 => {
                    let (left, middle) = node(data, next, depth + 1)?;
                    if middle != next + usize::from(value) {
                        return Err(Error::new("Invalid Help dictionary branch extent"));
                    }
                    let (right, end) = node(data, middle, depth + 1)?;
                    (left, right, end)
                }
                1 => {
                    let (left, end) = node(data, next, depth + 1)?;
                    (left, Node::Unit(value), end)
                }
                2 => {
                    let (right, end) = node(data, next, depth + 1)?;
                    (Node::Unit(value), right, end)
                }
                3 => (Node::Unit(value), Node::Unit(short(data, next)?), next + 2),
                _ => return Err(Error::new("Unknown Help dictionary node")),
            };
            Ok((Node::Branch(Box::new(left), Box::new(right)), end))
        }
        let (root, end) = node(data, 0, 0)?;
        if end != data.len() {
            return Err(Error::new("Trailing Help dictionary data"));
        }
        Ok(Self(root))
    }

    /// Strings are individually NUL-terminated and byte-aligned. Padding bits
    /// belong to the preceding string; decoding them as text corrupts the next
    /// title, paragraph or example. No Unicode scalar substitution is applied.
    pub fn spans(&self, data: &[u8], remaining: &mut usize) -> Result<Vec<Vec<u16>>> {
        self.decode(data, remaining, 0, 128, false)
    }

    /// Original routine30611218 skips one byte after every encoded NUL and
    /// discards CR. The string count comes from the group descriptor, not from
    /// a guessed image dimension. Topic spans use a different envelope.
    pub fn strings(
        &self,
        data: &[u8],
        count: usize,
        remaining: &mut usize,
    ) -> Result<Vec<Vec<u16>>> {
        let strings = self.decode(data, remaining, 1, count, true)?;
        if strings.len() != count {
            return Err(Error::new("Help string group count mismatch"));
        }
        Ok(strings)
    }

    fn decode(
        &self,
        data: &[u8],
        remaining: &mut usize,
        padding: usize,
        limit: usize,
        skip_cr: bool,
    ) -> Result<Vec<Vec<u16>>> {
        let mut at = 0;
        let mut spans = Vec::new();
        while at < data.len() * 8 {
            if spans.len() == limit {
                return Err(Error::new("Unexpected additional Help text span"));
            }
            let mut text = Vec::new();
            loop {
                let mut node = &self.0;
                while let Node::Branch(left, right) = node {
                    let byte = data
                        .get(at / 8)
                        .ok_or_else(|| Error::new("Unterminated Help text span"))?;
                    node = if byte & (1 << (at % 8)) == 0 {
                        left
                    } else {
                        right
                    };
                    at += 1;
                }
                let Node::Unit(unit) = node else {
                    unreachable!()
                };
                if *unit == 0 {
                    break;
                }
                *remaining = remaining
                    .checked_sub(1)
                    .ok_or_else(|| Error::new("Help text exceeds its decoded size limit"))?;
                if !skip_cr || *unit != 13 {
                    text.push(*unit);
                }
            }
            spans.push(text);
            at = (at.div_ceil(8) + padding) * 8;
        }
        if spans.is_empty() {
            return Err(Error::new("Help topic has no title"));
        }
        Ok(spans)
    }
}
