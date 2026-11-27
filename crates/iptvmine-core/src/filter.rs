use std::collections::HashSet;

const BLOCKED_CATEGORIES: &[&str] = &[
    "adult",
    "xxx",
    "18+",
    "18 +",
    "porn",
    "erotic",
    "sex",
    "sexy",
    "adults only",
    "mature",
    "nsfw",
    "x-rated",
    "r-rated",
    "adulto",
    "adulte",
    "erwachsene",
    "成人",
    "大人",
    "성인",
];

const BLOCKED_CHANNEL_KEYWORDS: &[&str] = &[
    "xxx", "porn", "adult", "sex", "erotic", "playboy", "hustler", "brazzers", "bangbros",
];

#[derive(Debug, Default, Clone)]
pub struct ContentFilter {
    custom_blocked_categories: HashSet<String>,
}

impl ContentFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_custom_blocked_category(&mut self, category: &str) {
        let normalized = category.trim().to_lowercase();
        if !normalized.is_empty() {
            self.custom_blocked_categories.insert(normalized);
        }
    }

    pub fn remove_custom_blocked_category(&mut self, category: &str) {
        self.custom_blocked_categories
            .remove(&category.trim().to_lowercase());
    }

    pub fn is_blocked_category(&self, category: Option<&str>) -> bool {
        let Some(cat) = category.map(str::trim).filter(|s| !s.is_empty()) else {
            return false;
        };
        let lower = cat.to_lowercase();
        self.custom_blocked_categories.contains(&lower)
            || BLOCKED_CATEGORIES
                .iter()
                .any(|blocked| lower == *blocked || lower.contains(blocked))
    }

    pub fn is_blocked_channel_name(&self, name: Option<&str>) -> bool {
        let Some(n) = name.map(str::trim).filter(|s| !s.is_empty()) else {
            return false;
        };
        let lower = n.to_lowercase();
        BLOCKED_CHANNEL_KEYWORDS
            .iter()
            .any(|kw| lower.contains(kw))
    }

    pub fn should_block_content(&self, channel_name: Option<&str>, category: Option<&str>) -> bool {
        self.is_blocked_category(category) || self.is_blocked_channel_name(channel_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_adult_categories_and_keywords() {
        let mut filter = ContentFilter::new();
        assert!(filter.should_block_content(Some("News HD"), Some("Adult 18+")));
        assert!(filter.should_block_content(Some("Playboy TV"), Some("Entertainment")));
        assert!(!filter.should_block_content(Some("Sports HD"), Some("Sports")));

        filter.add_custom_blocked_category("Gambling");
        assert!(filter.should_block_content(Some("Poker Channel"), Some("gambling")));
    }
}
