//! Shared command-palette matching for names, completion candidates and filters.
// Lower tuples sort first. Metadata matches always follow name-only matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MatchRank {
    metadata_words: usize,
    worst_tier: u8,
    total_tier: usize,
    penalty: i64,
    length: usize,
}

struct SearchText {
    chars: Vec<char>,
    boundaries: Vec<bool>,
}
impl SearchText {
    fn new(text: &str) -> Self {
        let original: Vec<_> = text.chars().collect();
        let mut chars = Vec::new();
        let mut boundaries = Vec::new();
        for (i, ch) in original.iter().copied().enumerate() {
            let boundary = i == 0 || !original[i - 1].is_alphanumeric()
                || (ch.is_uppercase() && (original[i - 1].is_lowercase()
                    || original.get(i + 1).is_some_and(|next| next.is_lowercase())));
            for (offset, lower) in ch.to_lowercase().enumerate() {
                chars.push(lower);
                boundaries.push(boundary && offset == 0);
            }
        }
        Self { chars, boundaries }
    }

    fn rank(&self, query: &[char]) -> Option<(u8, i64)> {
        if query.is_empty() || query.len() > self.chars.len() { return None; }
        if self.chars == query { return Some((0, 0)); }
        if self.chars.starts_with(query) { return Some((1, 0)); }
        // Prefer a word-boundary occurrence over an earlier inside-word hit.
        if let Some(tier) = self.chars.windows(query.len()).enumerate()
            .filter(|(_, window)| *window == query)
            .map(|(i, _)| if self.boundaries[i] { 2 } else { 3 }).min() {
            return Some((tier, 0));
        }
        // Reject non-matches before allocating the scoring rows. Catalogs often
        // contain many thousands of names with no possible ordered match.
        let mut matched = 0;
        for ch in &self.chars {
            if *ch == query[matched] {
                matched += 1;
                if matched == query.len() { break; }
            }
        }
        if matched != query.len() { return None; }
        // Ordered subsequences, scored with a linear pass per query character.
        // Reward word initials and contiguous runs, penalize skipped characters.
        // Keeping every possible endpoint avoids greedy abbreviation mismatches.
        const NONE: i64 = i64::MIN / 4;
        let mut previous = vec![NONE; self.chars.len()];
        for (qi, wanted) in query.iter().enumerate() {
            let mut current = vec![NONE; self.chars.len()];
            let mut best_gap = NONE;
            for (i, ch) in self.chars.iter().enumerate() {
                if i > 0 && previous[i - 1] != NONE {
                    best_gap = best_gap.max(previous[i - 1] + (i - 1) as i64);
                }
                if ch != wanted { continue; }
                let bonus = 1 + if self.boundaries[i] { 12 } else { 0 };
                if qi == 0 {
                    current[i] = bonus - i as i64;
                } else {
                    let gap = if best_gap == NONE { NONE } else { best_gap - i as i64 + 1 };
                    let contiguous = if i > 0 && previous[i - 1] != NONE { previous[i - 1] + 8 } else { NONE };
                    let best = gap.max(contiguous);
                    if best != NONE { current[i] = best + bonus; }
                }
            }
            previous = current;
        }
        previous.into_iter().max().filter(|score| *score != NONE).map(|score| (4, -score))
    }
}

pub(crate) struct Query {
    words: Vec<Vec<char>>,
    phrase: Vec<char>,
}
impl Query {
    pub(crate) fn new(query: &str) -> Self {
        let query = query.trim().to_lowercase();
        Self { words: query.split_whitespace().map(|word| word.chars().collect()).collect(), phrase: query.chars().collect() }
    }
    pub(crate) fn rank(&self, name: &str, metadata: &str) -> Option<MatchRank> {
        if self.words.is_empty() {
            return Some(MatchRank { metadata_words: 0, worst_tier: 0, total_tier: 0, penalty: 0, length: 0 });
        }
        let label = SearchText::new(name);
        let mut detail = None;
        let mut rank = MatchRank { metadata_words: 0, worst_tier: 0, total_tier: 0, penalty: 0, length: label.chars.len() };
        for word in &self.words {
            let (tier, penalty) = if let Some(found) = label.rank(word) { found }
                else { rank.metadata_words += 1; detail.get_or_insert_with(|| SearchText::new(metadata)).rank(word)? };
            rank.worst_tier = rank.worst_tier.max(tier);
            rank.total_tier += tier as usize;
            rank.penalty += penalty;
        }
        if rank.metadata_words == 0 && let Some((tier @ 0..=1, _)) = label.rank(&self.phrase) {
            rank.worst_tier = tier;
            rank.total_tier = tier as usize;
            rank.penalty = 0;
        }
        Some(rank)
    }
    pub(crate) fn ranked<T>(&self, items: impl IntoIterator<Item = T>, fields: impl Fn(&T) -> (&str, &str)) -> Vec<T> {
        let mut ranked: Vec<_> = items.into_iter().filter_map(|item| {
            let (name, detail) = fields(&item);
            self.rank(name, detail).map(|rank| (rank, item))
        }).collect();
        ranked.sort_by_key(|(rank, _)| *rank);
        ranked.into_iter().map(|(_, item)| item).collect()
    }
}
