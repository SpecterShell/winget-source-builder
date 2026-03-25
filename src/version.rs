use std::cmp::Ordering;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum ApproximateComparator {
    None,
    LessThan,
    GreaterThan,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct VersionPart {
    integer: u64,
    other: String,
    folded_other: String,
}

impl VersionPart {
    fn zero() -> Self {
        Self {
            integer: 0,
            other: String::new(),
            folded_other: String::new(),
        }
    }

    fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self::zero();
        }

        let digit_prefix_len = trimmed
            .bytes()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let (integer, other) = if digit_prefix_len == 0 {
            (0, trimmed.to_string())
        } else if digit_prefix_len == trimmed.len() {
            match trimmed.parse::<u64>() {
                Ok(integer) => (integer, String::new()),
                Err(_) => (0, trimmed.to_string()),
            }
        } else {
            let (prefix, suffix) = trimmed.split_at(digit_prefix_len);
            match prefix.parse::<u64>() {
                Ok(integer) => (integer, suffix.to_string()),
                Err(_) => (0, trimmed.to_string()),
            }
        };

        Self {
            integer,
            folded_other: other.to_ascii_lowercase(),
            other,
        }
    }
}

impl Ord for VersionPart {
    fn cmp(&self, other: &Self) -> Ordering {
        self.integer.cmp(&other.integer).then_with(|| {
            match (self.other.is_empty(), other.other.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.folded_other.cmp(&other.folded_other),
            }
        })
    }
}

impl PartialOrd for VersionPart {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct ParsedVersion {
    parts: Vec<VersionPart>,
    approximate: ApproximateComparator,
}

impl ParsedVersion {
    fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        let (approximate, mut base) = if let Some(rest) = trimmed.strip_prefix("< ") {
            (ApproximateComparator::LessThan, rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix("> ") {
            (ApproximateComparator::GreaterThan, rest.trim())
        } else {
            (ApproximateComparator::None, trimmed)
        };

        let digit_pos = base.find(|ch: char| ch.is_ascii_digit());
        let split_pos = base.find('.');
        if let Some(digit_pos) = digit_pos
            && split_pos.is_none_or(|split_pos| digit_pos < split_pos)
        {
            base = &base[digit_pos..];
        }

        let mut parts = base.split('.').map(VersionPart::parse).collect::<Vec<_>>();
        trim_trailing_zero_parts(&mut parts);

        Self { parts, approximate }
    }

    fn is_base_latest(&self) -> bool {
        self.parts.len() == 1
            && self.parts[0].integer == 0
            && self.parts[0].folded_other == "latest"
    }

    fn is_base_unknown(&self) -> bool {
        self.parts.len() == 1
            && self.parts[0].integer == 0
            && self.parts[0].folded_other == "unknown"
    }

    fn approximate_compare_less_than(&self, other: &Self) -> bool {
        (self.approximate == ApproximateComparator::LessThan
            && other.approximate != ApproximateComparator::LessThan)
            || (self.approximate == ApproximateComparator::None
                && other.approximate == ApproximateComparator::GreaterThan)
    }
}

fn trim_trailing_zero_parts(parts: &mut Vec<VersionPart>) {
    while parts
        .last()
        .is_some_and(|part| part.integer == 0 && part.other.is_empty())
    {
        parts.pop();
    }
}

pub(crate) fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = ParsedVersion::parse(left);
    let right = ParsedVersion::parse(right);

    match (left.is_base_latest(), right.is_base_latest()) {
        (true, true) => return approximate_ordering(&left, &right),
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }

    match (left.is_base_unknown(), right.is_base_unknown()) {
        (true, true) => return approximate_ordering(&left, &right),
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (false, false) => {}
    }

    let zero = VersionPart::zero();
    let max_len = left.parts.len().max(right.parts.len());
    for index in 0..max_len {
        let left_part = left.parts.get(index).unwrap_or(&zero);
        let right_part = right.parts.get(index).unwrap_or(&zero);
        let ordering = left_part.cmp(right_part);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    approximate_ordering(&left, &right)
}

fn approximate_ordering(left: &ParsedVersion, right: &ParsedVersion) -> Ordering {
    if left.approximate_compare_less_than(right) {
        Ordering::Less
    } else if right.approximate_compare_less_than(left) {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

pub(crate) fn compare_version_and_channel(
    left_version: &str,
    left_channel: &str,
    right_version: &str,
    right_channel: &str,
) -> Ordering {
    left_channel
        .cmp(right_channel)
        .then_with(|| compare_versions(right_version, left_version))
}

#[cfg(test)]
mod tests {
    use super::{compare_version_and_channel, compare_versions};

    #[test]
    fn trims_trailing_zero_parts_like_winget() {
        assert_eq!(compare_versions("1.0", "1.0.0"), std::cmp::Ordering::Equal);
        assert_eq!(compare_versions("2.0.0.0", "2"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn keeps_release_above_suffix_versions() {
        assert!(compare_versions("1.0.0", "1.0.0-preview.1").is_gt());
        assert!(compare_versions("1.0.0-preview.2", "1.0.0-preview.1").is_gt());
    }

    #[test]
    fn trims_leading_non_digit_prefixes() {
        assert_eq!(
            compare_versions("Version 1.2.0", "1.2"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn sorts_latest_and_unknown_sentinels() {
        assert!(compare_versions("Latest", "9.9.9").is_gt());
        assert!(compare_versions("Unknown", "0.0.1").is_lt());
    }

    #[test]
    fn sorts_version_and_channel_with_newer_versions_first_within_channel() {
        assert!(
            compare_version_and_channel("2.0.0", "", "1.0.0", "").is_lt(),
            "newer version should sort earlier within the same channel"
        );
        assert!(
            compare_version_and_channel("1.0.0", "beta", "9.0.0", "").is_gt(),
            "channel ordering should dominate version ordering"
        );
    }
}
