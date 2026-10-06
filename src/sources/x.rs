//! X (Twitter) source: turns a status URL into an `Article` via the FxTwitter API.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::article::{Article, Block, Span};
use crate::source::{Source, SourceError};

/// Hosts whose URLs we accept (all of them serve the same status paths).
const HOSTS: [&str; 7] = [
    "x.com",
    "twitter.com",
    "mobile.twitter.com",
    "www.x.com",
    "www.twitter.com",
    "fxtwitter.com",
    "fixupx.com",
];

/// The two parts of a status URL that the API needs.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusRef {
    pub user: String,
    pub id: String,
}

/// Extracts the user and status id from an X/Twitter status URL.
///
/// Returns `None` when the host is unsupported or the path is not `/{user}/status/{id}`.
pub fn parse_status_url(url: &str) -> Option<StatusRef> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    // Drop the query string and fragment, keep "host/path".
    let rest = rest.split(['?', '#']).next()?;
    let mut parts = rest.split('/');

    let host = parts.next()?;
    if !HOSTS.contains(&host) {
        return None;
    }

    let user = parts.next().filter(|user| !user.is_empty())?;
    if parts.next() != Some("status") {
        return None;
    }
    let id = parts.next().filter(|id| is_numeric_id(id))?;

    Some(StatusRef {
        user: user.to_string(),
        id: id.to_string(),
    })
}

fn is_numeric_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_digit())
}

/// Fetches X Articles through the FxTwitter API.
#[derive(Debug, Default)]
pub struct XSource;

impl XSource {
    /// Creates the source.
    pub fn new() -> Self {
        XSource
    }
}

impl Source for XSource {
    fn name(&self) -> &'static str {
        "x"
    }

    fn can_handle(&self, url: &str) -> bool {
        parse_status_url(url).is_some()
    }

    fn fetch(&self, url: &str) -> Result<Article, SourceError> {
        let status =
            parse_status_url(url).ok_or_else(|| SourceError::UnsupportedUrl(url.to_string()))?;
        let body = http_get(&format!(
            "https://api.fxtwitter.com/{}/status/{}",
            status.user, status.id
        ))
        .map_err(|reason| SourceError::Fetch {
            url: url.to_string(),
            reason,
        })?;
        parse_response(&body, url)
    }
}

/// Thin HTTP wrapper: returns the response body or a readable error message.
fn http_get(api_url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("x2k/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    client
        .get(api_url)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.text())
        .map_err(|e| e.to_string())
}

// ---- FxTwitter response shape (only the fields we need) ----

#[derive(Deserialize)]
struct Response {
    tweet: Option<Tweet>,
}

#[derive(Deserialize)]
struct Tweet {
    #[serde(default)]
    author: Author,
    article: Option<ArticleData>,
}

#[derive(Deserialize, Default)]
struct Author {
    #[serde(default)]
    name: String,
    #[serde(default)]
    screen_name: String,
}

#[derive(Deserialize)]
struct ArticleData {
    #[serde(default)]
    title: String,
    cover_media: Option<MediaRef>,
    content: Content,
    #[serde(default)]
    media_entities: Vec<MediaEntity>,
}

#[derive(Deserialize)]
struct MediaRef {
    media_info: MediaInfo,
}

#[derive(Deserialize)]
struct MediaInfo {
    #[serde(default)]
    original_img_url: String,
}

#[derive(Deserialize)]
struct MediaEntity {
    media_id: String,
    media_info: MediaInfo,
}

#[derive(Deserialize)]
struct Content {
    #[serde(default)]
    blocks: Vec<RawBlock>,
    #[serde(default, rename = "entityMap")]
    entity_map: Vec<EntityEntry>,
}

/// A Draft.js block. Offsets inside it count UTF-16 code units.
#[derive(Deserialize)]
struct RawBlock {
    #[serde(default)]
    text: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default, rename = "inlineStyleRanges")]
    style_ranges: Vec<StyleRange>,
    #[serde(default, rename = "entityRanges")]
    entity_ranges: Vec<EntityRange>,
}

#[derive(Deserialize)]
struct StyleRange {
    offset: usize,
    length: usize,
    style: String,
}

#[derive(Deserialize)]
struct EntityRange {
    key: usize,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    length: usize,
}

#[derive(Deserialize)]
struct EntityEntry {
    key: String,
    value: Entity,
}

#[derive(Deserialize)]
struct Entity {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    data: serde_json::Value,
}

/// Entities by their key, as the ranges refer to them.
type Entities<'a> = HashMap<&'a str, &'a Entity>;

/// Converts a FxTwitter JSON response into an `Article`.
pub fn parse_response(json: &str, url: &str) -> Result<Article, SourceError> {
    // FxTwitter answers missing posts with an HTML page or a JSON body without `tweet`.
    let not_found = || {
        SourceError::Parse(
            "could not find this post (it may not exist, be private, or FxTwitter may be down)"
                .to_string(),
        )
    };
    let response: Response = serde_json::from_str(json).map_err(|_| not_found())?;
    let tweet = response.tweet.ok_or_else(not_found)?;
    let article = tweet
        .article
        .ok_or_else(|| SourceError::Parse("this post is not an X Article".to_string()))?;

    let entities: Entities = article
        .content
        .entity_map
        .iter()
        .map(|entry| (entry.key.as_str(), &entry.value))
        .collect();
    let images: HashMap<&str, &str> = article
        .media_entities
        .iter()
        .map(|m| (m.media_id.as_str(), m.media_info.original_img_url.as_str()))
        .collect();

    Ok(Article {
        title: article.title,
        author: format!("{} (@{})", tweet.author.name, tweet.author.screen_name),
        url: url.to_string(),
        cover_image: article
            .cover_media
            .map(|cover| cover.media_info.original_img_url)
            .filter(|url| !url.is_empty()),
        blocks: convert_blocks(&article.content.blocks, &entities, &images),
    })
}

/// Maps Draft.js blocks to `Block`s, grouping consecutive list items.
fn convert_blocks(
    raw_blocks: &[RawBlock],
    entities: &Entities,
    images: &HashMap<&str, &str>,
) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();

    for raw in raw_blocks {
        if raw.kind == "atomic" {
            blocks.extend(convert_atomic(raw, entities, images));
            continue;
        }
        if raw.text.trim().is_empty() {
            continue;
        }
        let spans = build_spans(raw, entities);
        match raw.kind.as_str() {
            "unordered-list-item" => match blocks.last_mut() {
                Some(Block::BulletList(items)) => items.push(spans),
                _ => blocks.push(Block::BulletList(vec![spans])),
            },
            "ordered-list-item" => match blocks.last_mut() {
                Some(Block::NumberedList(items)) => items.push(spans),
                _ => blocks.push(Block::NumberedList(vec![spans])),
            },
            "blockquote" => blocks.push(Block::Quote(spans)),
            "header-one" => blocks.push(Block::Heading { level: 1, spans }),
            "header-two" => blocks.push(Block::Heading { level: 2, spans }),
            kind if kind.starts_with("header-") => blocks.push(Block::Heading { level: 3, spans }),
            _ => blocks.push(Block::Paragraph(spans)),
        }
    }
    blocks
}

/// Turns an `atomic` block (image or divider) into zero or more blocks.
fn convert_atomic(raw: &RawBlock, entities: &Entities, images: &HashMap<&str, &str>) -> Vec<Block> {
    let Some(entity) = raw
        .entity_ranges
        .first()
        .and_then(|range| entities.get(range.key.to_string().as_str()))
    else {
        return Vec::new();
    };

    if entity.kind == "MEDIA" {
        let items = entity.data["mediaItems"].as_array().into_iter().flatten();
        items
            .filter_map(|item| item["mediaId"].as_str())
            .filter_map(|media_id| images.get(media_id))
            .map(|url| Block::Image {
                url: url.to_string(),
                caption: None,
            })
            .collect()
    } else if entity.kind.contains("DIVIDER") {
        vec![Block::Divider]
    } else {
        Vec::new()
    }
}

/// Splits a block's text at every style/link boundary and formats each piece.
fn build_spans(raw: &RawBlock, entities: &Entities) -> Vec<Span> {
    let text = &raw.text;
    let to_bytes = |offset: usize, length: usize| {
        (
            utf16_to_byte(text, offset),
            utf16_to_byte(text, offset.saturating_add(length)),
        )
    };

    let styles: Vec<(usize, usize, &str)> = raw
        .style_ranges
        .iter()
        .map(|r| {
            let (start, end) = to_bytes(r.offset, r.length);
            (start, end, r.style.as_str())
        })
        .collect();
    let links: Vec<(usize, usize, &str)> = raw
        .entity_ranges
        .iter()
        .filter_map(|r| {
            let entity = entities.get(r.key.to_string().as_str())?;
            let url = entity.data["url"]
                .as_str()
                .filter(|_| entity.kind == "LINK")?;
            let (start, end) = to_bytes(r.offset, r.length);
            Some((start, end, url))
        })
        .collect();

    // Every place where formatting can change, plus the text edges.
    let mut cuts: Vec<usize> = vec![0, text.len()];
    cuts.extend(styles.iter().chain(&links).flat_map(|r| [r.0, r.1]));
    cuts.sort_unstable();
    cuts.dedup();

    cuts.windows(2)
        .filter_map(|pair| {
            let (from, to) = (pair[0], pair[1]);
            let covers = |range: &&(usize, usize, &str)| range.0 <= from && to <= range.1;
            let has_style = |name: &str| {
                styles
                    .iter()
                    .filter(covers)
                    .any(|(_, _, style)| *style == name)
            };
            Some(Span {
                text: text.get(from..to)?.to_string(),
                bold: has_style("Bold"),
                italic: has_style("Italic"),
                link: links.iter().find(covers).map(|(_, _, url)| url.to_string()),
            })
        })
        .filter(|span| !span.text.is_empty())
        .collect()
}

/// Converts a UTF-16 code-unit offset (as JavaScript counts) into a byte index.
///
/// Offsets past the end are clamped to the text length.
fn utf16_to_byte(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (index, c) in text.char_indices() {
        if seen >= units {
            return index;
        }
        seen += c.len_utf16();
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::article::{Block, Span};

    const FIXTURE: &str = include_str!("../../tests/fixtures/fxtwitter_article.json");
    const URL: &str = "https://x.com/thedankoe/status/2101361833940791607";

    #[test]
    fn parses_supported_status_urls() {
        let cases = [
            ("https://x.com/alice/status/123", "alice", "123"),
            ("http://twitter.com/alice/status/123", "alice", "123"),
            (
                "https://mobile.twitter.com/alice/status/123",
                "alice",
                "123",
            ),
            ("https://www.x.com/alice/status/123", "alice", "123"),
            ("https://www.twitter.com/alice/status/123", "alice", "123"),
            ("https://fxtwitter.com/alice/status/123", "alice", "123"),
            ("https://fixupx.com/alice/status/123", "alice", "123"),
            ("https://x.com/i/status/123", "i", "123"),
            ("https://x.com/alice/status/123/photo/1", "alice", "123"),
            ("https://x.com/alice/status/123?s=20&t=abc", "alice", "123"),
            ("https://x.com/alice/status/123#top", "alice", "123"),
            ("https://x.com/alice/status/123/", "alice", "123"),
        ];
        for (url, user, id) in cases {
            let parsed = parse_status_url(url);
            assert_eq!(
                parsed,
                Some(StatusRef {
                    user: user.into(),
                    id: id.into()
                }),
                "url: {url}"
            );
        }
    }

    #[test]
    fn rejects_unsupported_status_urls() {
        let cases = [
            "https://x.com/alice/status/abc",
            "https://example.com/alice/status/123",
            "https://x.com/alice/status",
            "https://x.com/alice/status/",
            "https://x.com/alice",
            "https://x.com/",
            "ftp://x.com/alice/status/123",
            "x.com/alice/status/123",
            "not a url",
            "",
        ];
        for url in cases {
            assert_eq!(parse_status_url(url), None, "url: {url}");
        }
    }

    #[test]
    fn can_handle_follows_url_parsing() {
        assert!(XSource::new().can_handle("https://x.com/a/status/1"));
        assert!(!XSource::new().can_handle("https://example.com/a/status/1"));
        assert_eq!(XSource::new().name(), "x");
    }

    fn all_spans(article: &Article) -> Vec<&Span> {
        let mut spans = Vec::new();
        for block in &article.blocks {
            match block {
                Block::Paragraph(s) | Block::Quote(s) => spans.extend(s),
                Block::Heading { spans: s, .. } => spans.extend(s),
                Block::BulletList(items) | Block::NumberedList(items) => {
                    spans.extend(items.iter().flatten())
                }
                Block::Image { .. } | Block::Divider => {}
            }
        }
        spans
    }

    #[test]
    fn maps_fixture_metadata() {
        let article = parse_response(FIXTURE, URL).unwrap();

        assert_eq!(article.title, "How to become disgustingly self-disciplined");
        assert!(article.author.contains("@thedankoe"));
        assert_eq!(article.url, URL);
        assert!(article.cover_image.is_some());
    }

    #[test]
    fn maps_fixture_blocks() {
        let article = parse_response(FIXTURE, URL).unwrap();

        let h2 = article
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Heading { level: 2, .. }))
            .count();
        let quotes = article
            .blocks
            .iter()
            .filter(|b| matches!(b, Block::Quote(_)))
            .count();
        assert_eq!(h2, 4);
        assert_eq!(quotes, 2);
        assert!(
            article
                .blocks
                .iter()
                .any(|b| matches!(b, Block::Image { url, .. } if url.contains("pbs.twimg.com")))
        );
    }

    #[test]
    fn maps_fixture_inline_styles() {
        let article = parse_response(FIXTURE, URL).unwrap();
        let spans = all_spans(&article);

        assert!(spans.iter().any(|s| s.italic));
        assert!(spans.iter().any(|s| s.bold));
        assert!(
            spans
                .iter()
                .any(|s| s.link.as_deref() == Some("https://eden.so/"))
        );
        assert!(spans.iter().all(|s| !s.text.is_empty()));
    }

    #[test]
    fn non_article_tweet_is_a_parse_error() {
        let json = r#"{"code":200,"tweet":{"url":"u","author":{"name":"A","screen_name":"a"}}}"#;

        let result = parse_response(json, URL);

        assert!(matches!(result, Err(SourceError::Parse(msg)) if msg.contains("not an X Article")));
    }

    #[test]
    fn invalid_json_is_a_parse_error() {
        assert!(matches!(
            parse_response("nope", URL),
            Err(SourceError::Parse(_))
        ));
    }

    #[test]
    fn html_page_instead_of_json_means_post_not_found() {
        let html = "<!DOCTYPE html><html><body>FxTwitter</body></html>";

        let result = parse_response(html, URL);

        assert!(
            matches!(result, Err(SourceError::Parse(msg)) if msg.contains("could not find this post"))
        );
    }

    #[test]
    fn json_without_tweet_means_post_not_found() {
        let json = r#"{"code":404,"message":"NOT_FOUND"}"#;

        let result = parse_response(json, URL);

        assert!(
            matches!(result, Err(SourceError::Parse(msg)) if msg.contains("could not find this post"))
        );
    }

    /// Builds a minimal FxTwitter response whose article holds the given blocks JSON.
    fn response_with_blocks(blocks: &str) -> String {
        format!(
            r#"{{"code":200,"tweet":{{"url":"u","author":{{"name":"A","screen_name":"a"}},
            "article":{{"title":"T","content":{{"blocks":{blocks},"entityMap":[]}}}}}}}}"#
        )
    }

    #[test]
    fn utf16_offsets_are_converted_for_emoji_text() {
        // "café 🚀 bold": the rocket is 2 UTF-16 units but 4 bytes, so "bold" starts at 8.
        let json = response_with_blocks(
            r#"[{"key":"a","text":"café 🚀 bold","type":"unstyled",
            "inlineStyleRanges":[{"offset":8,"length":4,"style":"Bold"}],"entityRanges":[]}]"#,
        );

        let article = parse_response(&json, URL).unwrap();

        assert_eq!(
            article.blocks,
            vec![Block::Paragraph(vec![
                Span::plain("café 🚀 "),
                Span {
                    bold: true,
                    ..Span::plain("bold")
                },
            ])]
        );
    }

    #[test]
    fn out_of_range_offsets_are_clamped() {
        let json = response_with_blocks(
            r#"[{"key":"a","text":"hi 🚀","type":"unstyled",
            "inlineStyleRanges":[{"offset":3,"length":50,"style":"Italic"},
            {"offset":99,"length":5,"style":"Bold"}],"entityRanges":[]}]"#,
        );

        let article = parse_response(&json, URL).unwrap();

        assert_eq!(
            article.blocks,
            vec![Block::Paragraph(vec![
                Span::plain("hi "),
                Span {
                    italic: true,
                    ..Span::plain("🚀")
                },
            ])]
        );
    }

    #[test]
    fn consecutive_list_items_are_grouped() {
        let json = response_with_blocks(
            r#"[
            {"key":"1","text":"a","type":"unordered-list-item","inlineStyleRanges":[],"entityRanges":[]},
            {"key":"2","text":"b","type":"unordered-list-item","inlineStyleRanges":[],"entityRanges":[]},
            {"key":"3","text":"mid","type":"unstyled","inlineStyleRanges":[],"entityRanges":[]},
            {"key":"4","text":"c","type":"ordered-list-item","inlineStyleRanges":[],"entityRanges":[]},
            {"key":"5","text":"d","type":"ordered-list-item","inlineStyleRanges":[],"entityRanges":[]}
            ]"#,
        );

        let article = parse_response(&json, URL).unwrap();

        assert_eq!(
            article.blocks,
            vec![
                Block::BulletList(vec![vec![Span::plain("a")], vec![Span::plain("b")]]),
                Block::Paragraph(vec![Span::plain("mid")]),
                Block::NumberedList(vec![vec![Span::plain("c")], vec![Span::plain("d")]]),
            ]
        );
    }

    #[test]
    fn headings_levels_and_empty_paragraphs() {
        let json = response_with_blocks(
            r#"[
            {"key":"1","text":"one","type":"header-one","inlineStyleRanges":[],"entityRanges":[]},
            {"key":"2","text":"two","type":"header-two","inlineStyleRanges":[],"entityRanges":[]},
            {"key":"3","text":"five","type":"header-five","inlineStyleRanges":[],"entityRanges":[]},
            {"key":"4","text":"  ","type":"unstyled","inlineStyleRanges":[],"entityRanges":[]},
            {"key":"5","text":"odd","type":"weird-type","inlineStyleRanges":[],"entityRanges":[]}
            ]"#,
        );

        let article = parse_response(&json, URL).unwrap();

        let heading = |level, text| Block::Heading {
            level,
            spans: vec![Span::plain(text)],
        };
        assert_eq!(
            article.blocks,
            vec![
                heading(1, "one"),
                heading(2, "two"),
                heading(3, "five"),
                Block::Paragraph(vec![Span::plain("odd")]),
            ]
        );
    }

    #[test]
    fn divider_and_unresolved_media_atomics() {
        let json = r#"{"code":200,"tweet":{"url":"u","author":{"name":"A","screen_name":"a"},
            "article":{"title":"T","content":{"blocks":[
              {"key":"1","text":" ","type":"atomic","inlineStyleRanges":[],"entityRanges":[{"key":0,"offset":0,"length":1}]},
              {"key":"2","text":" ","type":"atomic","inlineStyleRanges":[],"entityRanges":[{"key":1,"offset":0,"length":1}]}
            ],"entityMap":[
              {"key":"0","value":{"type":"MARKDOWN_DIVIDER","data":{}}},
              {"key":"1","value":{"type":"MEDIA","data":{"mediaItems":[{"mediaId":"nope"}]}}}
            ]}}}}"#;

        let article = parse_response(json, URL).unwrap();

        assert_eq!(article.blocks, vec![Block::Divider]);
    }

    /// Hits the real FxTwitter API. Run manually: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_fetch_of_a_real_article() {
        let article = XSource::new().fetch(URL).unwrap();

        assert_eq!(article.title, "How to become disgustingly self-disciplined");
        assert!(!article.blocks.is_empty());
    }
}
