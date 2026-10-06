//! Turns an `Article` into an EPUB that Kindle can read.
//!
//! Kindle cannot load remote images, so they are downloaded first and embedded.
//! The work is split in small steps: collect URLs, download, build XHTML, zip it.

use std::collections::HashMap;
use std::time::Duration;

use epub_builder::{EpubBuilder, EpubContent, ZipLibrary};

use crate::article::{Article, Block, Span};
use crate::cover::make_kindle_cover;

/// Minimal, Kindle-friendly stylesheet.
const CSS: &str = "body { line-height: 1.4; }\n\
h1 { font-size: 1.6em; margin-bottom: 0.2em; }\n\
.meta { color: #555; font-size: 0.9em; margin-bottom: 1.5em; }\n\
blockquote { margin: 1em 1.5em; font-style: italic; }\n\
img { max-width: 100%; }\n\
figure { margin: 1em 0; text-align: center; }\n\
figcaption { font-size: 0.85em; color: #555; }\n";

/// Where the chapter lives inside the EPUB.
const CHAPTER_FILE: &str = "article.xhtml";

/// Everything that can go wrong while building the EPUB.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("failed to build the EPUB: {0}")]
    Epub(String),
}

/// A downloaded image, ready to be embedded.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// Image URLs in reading order: the cover first, then image blocks. No duplicates.
pub fn collect_image_urls(article: &Article) -> Vec<String> {
    let block_urls = article.blocks.iter().filter_map(|block| match block {
        Block::Image { url, .. } => Some(url),
        _ => None,
    });

    let mut urls: Vec<String> = Vec::new();
    for url in article.cover_image.iter().chain(block_urls) {
        if !urls.contains(url) {
            urls.push(url.clone());
        }
    }
    urls
}

/// Downloads every URL. Failed downloads are left out of the result.
pub fn download_images(urls: &[String]) -> HashMap<String, Image> {
    let Ok(client) = reqwest::blocking::Client::builder()
        .user_agent(concat!("x2k/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()
    else {
        return HashMap::new();
    };

    let mut images = HashMap::new();
    for url in urls {
        if let Some(image) = download_image(&client, url) {
            images.insert(url.clone(), image);
        }
    }
    images
}

/// Downloads one image, or returns `None` if anything goes wrong.
fn download_image(client: &reqwest::blocking::Client, url: &str) -> Option<Image> {
    let response = client.get(url).send().ok()?.error_for_status().ok()?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = response.bytes().ok()?.to_vec();
    Some(Image {
        bytes,
        mime: pick_mime(content_type.as_deref(), url),
    })
}

/// Chooses the image MIME type: the Content-Type header, then the URL extension, then JPEG.
fn pick_mime(content_type: Option<&str>, url: &str) -> String {
    let from_header = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| value.starts_with("image/"));
    if let Some(mime) = from_header {
        return mime.to_string();
    }

    let path = url.split(['?', '#']).next().unwrap_or(url);
    let mime = match path
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "image/jpeg",
    };
    mime.to_string()
}

/// File extension for a MIME type, used to name embedded images.
fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => "jpg",
    }
}

/// Escapes text for XHTML (works for element text and attribute values).
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Renders spans, nesting the tags as `<a><strong><em>text</em></strong></a>`.
fn render_spans(spans: &[Span]) -> String {
    spans.iter().map(render_span).collect()
}

fn render_span(span: &Span) -> String {
    let mut html = escape(&span.text);
    if span.italic {
        html = format!("<em>{html}</em>");
    }
    if span.bold {
        html = format!("<strong>{html}</strong>");
    }
    match span.link.as_deref().filter(|link| is_safe_link(link)) {
        Some(link) => format!("<a href=\"{}\">{html}</a>", escape(link)),
        None => html,
    }
}

/// Only web and mail links are kept; anything else (e.g. `javascript:`) is dropped.
fn is_safe_link(link: &str) -> bool {
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| link.starts_with(scheme))
}

fn render_block(block: &Block, image_paths: &HashMap<String, String>) -> String {
    match block {
        Block::Paragraph(spans) => format!("<p>{}</p>", render_spans(spans)),
        Block::Heading { level, spans } => {
            // The article title is the h1, so article headings start at h2.
            let tag = (*level).max(1).saturating_add(1).min(6);
            format!("<h{tag}>{}</h{tag}>", render_spans(spans))
        }
        Block::Quote(spans) => format!("<blockquote><p>{}</p></blockquote>", render_spans(spans)),
        Block::BulletList(items) => format!("<ul>{}</ul>", render_items(items)),
        Block::NumberedList(items) => format!("<ol>{}</ol>", render_items(items)),
        Block::Image { url, caption } => render_image(url, caption.as_deref(), image_paths),
        Block::Divider => "<hr/>".to_string(),
    }
}

fn render_items(items: &[Vec<Span>]) -> String {
    items
        .iter()
        .map(|spans| format!("<li>{}</li>", render_spans(spans)))
        .collect()
}

/// An image block, or nothing when the image could not be downloaded.
fn render_image(url: &str, caption: Option<&str>, image_paths: &HashMap<String, String>) -> String {
    let Some(path) = image_paths.get(url) else {
        return String::new();
    };
    let caption = caption.filter(|text| !text.is_empty());
    let alt = escape(caption.unwrap_or(""));
    let figcaption = caption
        .map(|text| format!("<figcaption>{}</figcaption>", escape(text)))
        .unwrap_or_default();
    format!(
        "<figure><img src=\"{}\" alt=\"{alt}\"/>{figcaption}</figure>",
        escape(path)
    )
}

/// Builds the XHTML chapter. `image_paths` maps an image URL to its path inside the EPUB.
pub fn render_chapter_xhtml(article: &Article, image_paths: &HashMap<String, String>) -> String {
    let title = escape(&article.title);
    let body: Vec<String> = article
        .blocks
        .iter()
        .map(|block| render_block(block, image_paths))
        .filter(|html| !html.is_empty())
        .collect();

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE html>\n\
<html xmlns=\"http://www.w3.org/1999/xhtml\" lang=\"en\">\n\
<head>\n<meta charset=\"utf-8\"/>\n<title>{title}</title>\n\
<link rel=\"stylesheet\" type=\"text/css\" href=\"stylesheet.css\"/>\n</head>\n\
<body>\n<h1>{title}</h1>\n\
<p class=\"meta\">{author} - <a href=\"{url}\">{url}</a></p>\n{body}\n</body>\n</html>\n",
        author = escape(&article.author),
        url = escape(&article.url),
        body = body.join("\n"),
    )
}

/// Builds the EPUB file in memory. Pure: images come from the `images` map.
pub fn render_epub(
    article: &Article,
    images: &HashMap<String, Image>,
) -> Result<Vec<u8>, RenderError> {
    build_epub(article, images).map_err(|e| RenderError::Epub(e.to_string()))
}

/// The image to embed as the cover: a Kindle-friendly portrait version when
/// possible, otherwise the original.
fn cover_for_epub(image: &Image) -> Image {
    match make_kindle_cover(&image.bytes) {
        Some(jpeg) => Image {
            bytes: jpeg,
            mime: "image/jpeg".into(),
        },
        None => image.clone(),
    }
}

fn build_epub(article: &Article, images: &HashMap<String, Image>) -> epub_builder::Result<Vec<u8>> {
    let mut builder = EpubBuilder::new(ZipLibrary::new()?)?;
    builder.metadata("title", article.title.as_str())?;
    builder.metadata("author", article.author.as_str())?;
    builder.metadata("lang", "en")?;
    builder.stylesheet(CSS.as_bytes())?;

    // Embed each downloaded image as images/img-<n>.<ext>; the cover is flagged as such.
    let mut image_paths = HashMap::new();
    for (index, url) in collect_image_urls(article).iter().enumerate() {
        let Some(image) = images.get(url) else {
            continue;
        };
        let is_cover = article.cover_image.as_ref() == Some(url);
        let cover;
        let image = if is_cover {
            cover = cover_for_epub(image);
            &cover
        } else {
            image
        };
        let path = format!("images/img-{index}.{}", extension_for(&image.mime));
        if is_cover {
            builder.add_cover_image(&path, image.bytes.as_slice(), image.mime.as_str())?;
        } else {
            builder.add_resource(&path, image.bytes.as_slice(), image.mime.as_str())?;
        }
        image_paths.insert(url.clone(), path);
    }

    let chapter = render_chapter_xhtml(article, &image_paths);
    builder.add_content(
        EpubContent::new(CHAPTER_FILE, chapter.as_bytes()).title(article.title.as_str()),
    )?;

    let mut bytes = Vec::new();
    builder.generate(&mut bytes)?;
    Ok(bytes)
}

/// Longest slug we keep, so file names stay short.
const MAX_SLUG_LEN: usize = 60;

/// A safe file name for the EPUB, e.g. "my-title.epub".
pub fn file_name(article: &Article) -> String {
    let mut slug = String::new();
    for c in article.title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    // The slug is pure ASCII, so cutting by bytes is safe.
    slug.truncate(MAX_SLUG_LEN);
    let slug = slug.trim_matches('-');

    if slug.is_empty() {
        "article.epub".to_string()
    } else {
        format!("{slug}.epub")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::x::parse_response;

    const FIXTURE: &str = include_str!("../tests/fixtures/fxtwitter_article.json");

    fn article(blocks: Vec<Block>) -> Article {
        Article {
            title: "My Title".into(),
            author: "Ann".into(),
            url: "https://x.com/ann/status/1".into(),
            cover_image: None,
            blocks,
        }
    }

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Span::plain(text)])
    }

    fn image(url: &str) -> Block {
        Block::Image {
            url: url.into(),
            caption: None,
        }
    }

    fn paths(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(url, path)| (url.to_string(), path.to_string()))
            .collect()
    }

    #[test]
    fn collects_cover_first_then_images_without_duplicates() {
        let mut a = article(vec![
            image("https://i/b.jpg"),
            para("x"),
            image("https://i/a.jpg"),
            image("https://i/b.jpg"),
        ]);
        a.cover_image = Some("https://i/a.jpg".into());

        assert_eq!(
            collect_image_urls(&a),
            vec!["https://i/a.jpg", "https://i/b.jpg"]
        );
    }

    #[test]
    fn collects_nothing_when_there_are_no_images() {
        assert!(collect_image_urls(&article(vec![para("x")])).is_empty());
    }

    #[test]
    fn mime_prefers_content_type_header() {
        assert_eq!(
            pick_mime(Some("image/png; charset=binary"), "https://i/a.jpg"),
            "image/png"
        );
    }

    #[test]
    fn mime_falls_back_to_extension_then_jpeg() {
        assert_eq!(pick_mime(None, "https://i/a.png?x=1"), "image/png");
        assert_eq!(pick_mime(Some("text/html"), "https://i/a.gif"), "image/gif");
        assert_eq!(pick_mime(None, "https://i/a"), "image/jpeg");
    }

    #[test]
    fn chapter_has_title_author_and_source_link() {
        let xhtml = render_chapter_xhtml(&article(vec![]), &HashMap::new());

        assert!(xhtml.contains("<h1>My Title</h1>"));
        assert!(xhtml.contains("Ann"));
        assert!(xhtml.contains("<a href=\"https://x.com/ann/status/1\">"));
    }

    #[test]
    fn chapter_renders_text_blocks() {
        let a = article(vec![
            para("Hello"),
            Block::Heading {
                level: 1,
                spans: vec![Span::plain("Top")],
            },
            Block::Heading {
                level: 3,
                spans: vec![Span::plain("Deep")],
            },
            Block::Quote(vec![Span::plain("Wise")]),
            Block::BulletList(vec![vec![Span::plain("one")], vec![Span::plain("two")]]),
            Block::NumberedList(vec![vec![Span::plain("first")]]),
            Block::Divider,
        ]);

        let xhtml = render_chapter_xhtml(&a, &HashMap::new());

        assert!(xhtml.contains("<p>Hello</p>"));
        assert!(xhtml.contains("<h2>Top</h2>"));
        assert!(xhtml.contains("<h4>Deep</h4>"));
        assert!(xhtml.contains("<blockquote><p>Wise</p></blockquote>"));
        assert!(xhtml.contains("<ul><li>one</li><li>two</li></ul>"));
        assert!(xhtml.contains("<ol><li>first</li></ol>"));
        assert!(xhtml.contains("<hr/>"));
    }

    #[test]
    fn chapter_nests_inline_formatting() {
        let span = Span {
            text: "both".into(),
            bold: true,
            italic: true,
            link: Some("https://example.com/?a=1&b=2".into()),
        };

        let xhtml = render_chapter_xhtml(
            &article(vec![Block::Paragraph(vec![span])]),
            &HashMap::new(),
        );

        assert!(xhtml.contains(
            "<a href=\"https://example.com/?a=1&amp;b=2\"><strong><em>both</em></strong></a>"
        ));
    }

    #[test]
    fn chapter_drops_unsafe_link_schemes() {
        let span = Span {
            text: "click".into(),
            bold: false,
            italic: false,
            link: Some("javascript:alert(1)".into()),
        };

        let xhtml = render_chapter_xhtml(
            &article(vec![Block::Paragraph(vec![span])]),
            &HashMap::new(),
        );

        assert!(xhtml.contains("<p>click</p>"));
        assert!(!xhtml.contains("javascript"));
    }

    #[test]
    fn chapter_escapes_text_and_attributes() {
        let mut a = article(vec![para("<script>alert('x')</script> & \"q\"")]);
        a.title = "A & <B>".into();

        let xhtml = render_chapter_xhtml(&a, &HashMap::new());

        assert!(!xhtml.contains("<script>"));
        assert!(
            xhtml.contains("&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;q&quot;")
        );
        assert!(xhtml.contains("<h1>A &amp; &lt;B&gt;</h1>"));
    }

    #[test]
    fn chapter_embeds_known_images_with_caption_and_skips_missing_ones() {
        let a = article(vec![
            Block::Image {
                url: "https://i/a.jpg".into(),
                caption: Some("A cat".into()),
            },
            image("https://i/missing.jpg"),
        ]);

        let xhtml = render_chapter_xhtml(&a, &paths(&[("https://i/a.jpg", "images/img-0.jpg")]));

        assert!(xhtml.contains("<img src=\"images/img-0.jpg\" alt=\"A cat\"/>"));
        assert!(xhtml.contains("<figcaption>A cat</figcaption>"));
        assert!(!xhtml.contains("missing"));
    }

    #[test]
    fn epub_is_a_zip_with_the_epub_mimetype() {
        let bytes = render_epub(&article(vec![para("Hi")]), &HashMap::new()).unwrap();

        assert!(bytes.starts_with(b"PK"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("mimetypeapplication/epub+zip"));
    }

    #[test]
    fn epub_embeds_images_and_cover() {
        let mut a = article(vec![image("https://i/b.png")]);
        a.cover_image = Some("https://i/a.jpg".into());
        let mut images = HashMap::new();
        for (url, mime) in [
            ("https://i/a.jpg", "image/jpeg"),
            ("https://i/b.png", "image/png"),
        ] {
            images.insert(
                url.to_string(),
                Image {
                    bytes: vec![1, 2, 3],
                    mime: mime.into(),
                },
            );
        }

        let bytes = render_epub(&a, &images).unwrap();

        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("images/img-0.jpg"));
        assert!(text.contains("images/img-1.png"));
    }

    /// A PNG of the given size, as downloaded bytes.
    fn png_image(width: u32, height: u32) -> Image {
        let img = image::RgbImage::from_pixel(width, height, image::Rgb([9, 9, 9]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        Image {
            bytes: out.into_inner(),
            mime: "image/png".into(),
        }
    }

    #[test]
    fn wide_cover_becomes_a_portrait_jpeg() {
        let cover = cover_for_epub(&png_image(1920, 768));

        assert_eq!(cover.mime, "image/jpeg");
        let decoded = image::load_from_memory(&cover.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1600, 2560));
    }

    #[test]
    fn portrait_or_broken_cover_is_kept_as_is() {
        let portrait = png_image(800, 1280);
        assert_eq!(cover_for_epub(&portrait), portrait);

        let broken = Image {
            bytes: vec![1, 2, 3],
            mime: "image/jpeg".into(),
        };
        assert_eq!(cover_for_epub(&broken), broken);
    }

    #[test]
    fn epub_cover_of_a_wide_png_gets_a_jpg_path() {
        let mut a = article(vec![para("Hi")]);
        a.cover_image = Some("https://i/wide.png".into());
        let mut images = HashMap::new();
        images.insert("https://i/wide.png".to_string(), png_image(1920, 768));

        let bytes = render_epub(&a, &images).unwrap();

        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("images/img-0.jpg"));
        assert!(!text.contains("images/img-0.png"));
    }

    #[test]
    fn renders_the_fixture_article_without_network() {
        let a = parse_response(
            FIXTURE,
            "https://x.com/thedankoe/status/2101361833940791607",
        )
        .unwrap();

        let bytes = render_epub(&a, &HashMap::new()).unwrap();

        assert!(bytes.starts_with(b"PK"));
        assert!(bytes.len() > 1000);
    }

    #[test]
    fn file_name_is_a_slug() {
        let mut a = article(vec![]);
        a.title = "  Hello, World!! It's 2026 -- ok?  ".into();

        assert_eq!(file_name(&a), "hello-world-it-s-2026-ok.epub");
    }

    #[test]
    fn file_name_falls_back_and_truncates() {
        let mut a = article(vec![]);
        a.title = "¡¿日本語?!".into();
        assert_eq!(file_name(&a), "article.epub");

        a.title = "word ".repeat(40);
        let name = file_name(&a);
        assert!(name.len() <= 60 + ".epub".len());
        assert!(!name.contains("--"));
        assert!(!name.trim_end_matches(".epub").ends_with('-'));
    }
}
