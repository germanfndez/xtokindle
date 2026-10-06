//! Where articles come from: the `Source` trait and a registry that picks one per URL.

use crate::article::Article;

/// Everything that can go wrong while fetching an article.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("unsupported URL: {0}")]
    UnsupportedUrl(String),
    #[error("failed to fetch {url}: {reason}")]
    Fetch { url: String, reason: String },
    #[error("unexpected response: {0}")]
    Parse(String),
}

/// A place articles can be fetched from (X, Substack, ...).
pub trait Source {
    /// Short human-readable name, e.g. "x".
    fn name(&self) -> &'static str;
    /// Returns true if this source knows how to fetch the given URL.
    fn can_handle(&self, url: &str) -> bool;
    /// Downloads the article and converts it to the shared `Article` model.
    fn fetch(&self, url: &str) -> Result<Article, SourceError>;
}

/// Picks the right `Source` for a URL.
pub struct SourceRegistry {
    sources: Vec<Box<dyn Source>>,
}

impl SourceRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        SourceRegistry {
            sources: Vec::new(),
        }
    }

    /// Adds a source. Sources registered first win when several match.
    pub fn register(&mut self, source: Box<dyn Source>) {
        self.sources.push(source);
    }

    /// Returns the first source that can handle the URL.
    pub fn find(&self, url: &str) -> Result<&dyn Source, SourceError> {
        self.sources
            .iter()
            .find(|source| source.can_handle(url))
            .map(|source| source.as_ref())
            .ok_or_else(|| SourceError::UnsupportedUrl(url.to_string()))
    }
}

impl Default for SourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Handles any URL starting with `prefix`; its title tells tests which one answered.
    struct FakeSource {
        prefix: &'static str,
    }

    impl Source for FakeSource {
        fn name(&self) -> &'static str {
            self.prefix
        }

        fn can_handle(&self, url: &str) -> bool {
            url.starts_with(self.prefix)
        }

        fn fetch(&self, url: &str) -> Result<Article, SourceError> {
            Ok(Article {
                title: self.prefix.to_string(),
                author: String::new(),
                url: url.to_string(),
                cover_image: None,
                blocks: vec![],
            })
        }
    }

    fn registry(prefixes: &[&'static str]) -> SourceRegistry {
        let mut registry = SourceRegistry::new();
        for prefix in prefixes {
            registry.register(Box::new(FakeSource { prefix }));
        }
        registry
    }

    #[test]
    fn finds_the_matching_source() {
        let registry = registry(&["https://a.com", "https://b.com"]);

        let source = registry.find("https://b.com/post").unwrap();

        assert_eq!(source.name(), "https://b.com");
    }

    #[test]
    fn returns_first_match_when_several_match() {
        let registry = registry(&["https://a.com", "https://a.com/special"]);

        let source = registry.find("https://a.com/special/1").unwrap();

        assert_eq!(source.name(), "https://a.com");
    }

    #[test]
    fn returns_unsupported_url_when_nothing_matches() {
        let registry = registry(&["https://a.com"]);

        let result = registry.find("https://other.com");

        assert!(
            matches!(result, Err(SourceError::UnsupportedUrl(url)) if url == "https://other.com")
        );
    }

    #[test]
    fn empty_registry_supports_nothing() {
        assert!(SourceRegistry::default().find("https://a.com").is_err());
    }

    #[test]
    fn unsupported_url_message_includes_the_url() {
        let error = SourceError::UnsupportedUrl("https://other.com".into());

        assert_eq!(error.to_string(), "unsupported URL: https://other.com");
    }
}
