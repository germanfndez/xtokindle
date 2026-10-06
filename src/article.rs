//! Source-agnostic article model shared by every `Source` and the EPUB renderer.

/// A fetched article, independent of where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Article {
    pub title: String,
    pub author: String,
    pub url: String,
    pub cover_image: Option<String>,
    pub blocks: Vec<Block>,
}

/// One block-level piece of content.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Quote(Vec<Span>),
    BulletList(Vec<Vec<Span>>),
    NumberedList(Vec<Vec<Span>>),
    Image {
        url: String,
        caption: Option<String>,
    },
    Divider,
}

/// A run of text with inline formatting.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub link: Option<String>,
}

impl Span {
    /// Creates a span with no bold, italic or link.
    pub fn plain(text: impl Into<String>) -> Span {
        Span {
            text: text.into(),
            bold: false,
            italic: false,
            link: None,
        }
    }
}

impl Article {
    /// Counts whitespace-separated words in the text blocks (image captions are ignored).
    pub fn word_count(&self) -> usize {
        self.blocks.iter().map(Block::word_count).sum()
    }
}

impl Block {
    fn word_count(&self) -> usize {
        match self {
            Block::Paragraph(spans) | Block::Quote(spans) => count_words(spans),
            Block::Heading { spans, .. } => count_words(spans),
            Block::BulletList(items) | Block::NumberedList(items) => {
                items.iter().map(|spans| count_words(spans)).sum()
            }
            Block::Image { .. } | Block::Divider => 0,
        }
    }
}

/// Joins the spans' text and counts the words, so a word split across spans stays one word.
fn count_words(spans: &[Span]) -> usize {
    let text: String = spans.iter().map(|span| span.text.as_str()).collect();
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article(blocks: Vec<Block>) -> Article {
        Article {
            title: "T".into(),
            author: "A".into(),
            url: "https://example.com".into(),
            cover_image: None,
            blocks,
        }
    }

    #[test]
    fn plain_span_has_no_formatting() {
        let span = Span::plain("hello");

        assert_eq!(span.text, "hello");
        assert!(!span.bold);
        assert!(!span.italic);
        assert_eq!(span.link, None);
    }

    #[test]
    fn word_count_of_empty_article_is_zero() {
        assert_eq!(article(vec![]).word_count(), 0);
    }

    #[test]
    fn word_count_counts_text_blocks_across_spans() {
        let a = article(vec![
            Block::Paragraph(vec![Span::plain("one two "), Span::plain("three")]),
            Block::Heading {
                level: 2,
                spans: vec![Span::plain("four five")],
            },
            Block::Quote(vec![Span::plain("six")]),
            Block::BulletList(vec![
                vec![Span::plain("seven")],
                vec![Span::plain("eight nine")],
            ]),
            Block::NumberedList(vec![vec![Span::plain("ten")]]),
        ]);

        assert_eq!(a.word_count(), 10);
    }

    #[test]
    fn word_count_ignores_images_and_dividers() {
        let a = article(vec![
            Block::Image {
                url: "u".into(),
                caption: Some("a long caption here".into()),
            },
            Block::Divider,
        ]);

        assert_eq!(a.word_count(), 0);
    }
}
