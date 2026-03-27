use std::path::Path;

use anyhow::Error;
use rust_i18n::t;

#[derive(Debug, Clone)]
pub(crate) struct Messages {
    locale: String,
}

impl Messages {
    pub(crate) fn new(locale: impl AsRef<str>) -> Self {
        Self {
            locale: normalize_locale(locale.as_ref()),
        }
    }

    pub(crate) fn build_started(&self, repo: &Path, state: &Path) -> String {
        t!(
            "build.started",
            locale = self.locale.as_str(),
            repo = display_path(repo),
            state = display_path(state)
        )
        .to_string()
    }

    pub(crate) fn publish_started(&self, state: &Path, out: &Path) -> String {
        t!(
            "publish.started",
            locale = self.locale.as_str(),
            state = display_path(state),
            out = display_path(out)
        )
        .to_string()
    }

    pub(crate) fn scanning_repository(&self, repo: &Path) -> String {
        t!(
            "build.scanning_repository",
            locale = self.locale.as_str(),
            repo = display_path(repo)
        )
        .to_string()
    }

    pub(crate) fn dirty_versions_detected(&self, count: usize) -> String {
        t!(
            "build.dirty_versions_detected",
            locale = self.locale.as_str(),
            count = count
        )
        .to_string()
    }

    pub(crate) fn progress_scanning_files(&self) -> String {
        t!("progress.scanning_files", locale = self.locale.as_str()).to_string()
    }

    pub(crate) fn progress_hashing_files(&self) -> String {
        t!("progress.hashing_files", locale = self.locale.as_str()).to_string()
    }

    pub(crate) fn progress_computing_versions(&self) -> String {
        t!("progress.computing_versions", locale = self.locale.as_str()).to_string()
    }

    pub(crate) fn progress_staging_manifests(&self) -> String {
        t!("progress.staging_manifests", locale = self.locale.as_str()).to_string()
    }

    pub(crate) fn progress_running_adapter(&self, package_name: &str) -> String {
        t!(
            "progress.running_adapter",
            locale = self.locale.as_str(),
            package_name = package_name
        )
        .to_string()
    }

    pub(crate) fn progress_running_rust_backend(&self, package_name: &str) -> String {
        t!(
            "progress.running_rust_backend",
            locale = self.locale.as_str(),
            package_name = package_name
        )
        .to_string()
    }

    pub(crate) fn progress_committing_output(&self) -> String {
        t!("progress.committing_output", locale = self.locale.as_str()).to_string()
    }

    pub(crate) fn progress_packaging_publish(&self) -> String {
        t!("progress.packaging_publish", locale = self.locale.as_str()).to_string()
    }

    pub(crate) fn validation_queue_written(&self, count: usize, path: &Path) -> String {
        t!(
            "build.validation_queue_written",
            locale = self.locale.as_str(),
            count = count,
            path = display_path(path)
        )
        .to_string()
    }

    pub(crate) fn no_semantic_changes(&self) -> String {
        t!("build.no_semantic_changes", locale = self.locale.as_str()).to_string()
    }

    pub(crate) fn staging_publish_tree(&self, changed_versions: usize) -> String {
        t!(
            "build.staging_publish_tree",
            locale = self.locale.as_str(),
            changed_versions = changed_versions
        )
        .to_string()
    }

    pub(crate) fn running_adapter(&self, package_name: &str) -> String {
        t!(
            "build.running_adapter",
            locale = self.locale.as_str(),
            package_name = package_name
        )
        .to_string()
    }

    pub(crate) fn running_rust_backend(&self, package_name: &str) -> String {
        t!(
            "build.running_rust_backend",
            locale = self.locale.as_str(),
            package_name = package_name
        )
        .to_string()
    }

    pub(crate) fn build_staged(&self, stage: &Path, state: &Path) -> String {
        t!(
            "build.staged",
            locale = self.locale.as_str(),
            stage = display_path(stage),
            state = display_path(state)
        )
        .to_string()
    }

    pub(crate) fn publish_completed(&self, out: &Path, state: &Path) -> String {
        t!(
            "publish.completed",
            locale = self.locale.as_str(),
            out = display_path(out),
            state = display_path(state)
        )
        .to_string()
    }

    pub(crate) fn build_failed(&self, error: &Error) -> String {
        t!(
            "build.failed",
            locale = self.locale.as_str(),
            error = format!("{error:#}")
        )
        .to_string()
    }

    pub(crate) fn warning_numeric_package_version(
        &self,
        manifest_path: &Path,
        package_version: &str,
    ) -> String {
        t!(
            "warning.numeric_package_version",
            locale = self.locale.as_str(),
            manifest_path = display_path(manifest_path),
            package_version = package_version
        )
        .to_string()
    }

    pub(crate) fn warning_display_version_conflict(
        &self,
        package_id: &str,
        display_version: &str,
        retained_version: &str,
        stripped_versions: &str,
    ) -> String {
        t!(
            "warning.display_version_conflict",
            locale = self.locale.as_str(),
            package_id = package_id,
            display_version = display_version,
            retained_version = retained_version,
            stripped_versions = stripped_versions
        )
        .to_string()
    }

    #[cfg(test)]
    fn locale(&self) -> &str {
        &self.locale
    }
}

fn normalize_locale(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "en".to_string();
    }

    let base = trimmed
        .split('.')
        .next()
        .unwrap_or(trimmed)
        .split('@')
        .next()
        .unwrap_or(trimmed)
        .replace('_', "-");

    let normalized = base
        .split('-')
        .filter(|segment| !segment.is_empty())
        .enumerate()
        .map(|(index, segment)| normalize_locale_segment(index, segment))
        .collect::<Vec<_>>()
        .join("-");

    if normalized.is_empty() {
        "en".to_string()
    } else {
        normalized
    }
}

fn normalize_locale_segment(index: usize, segment: &str) -> String {
    if index == 0 {
        return segment.to_ascii_lowercase();
    }

    if segment.len() == 4 && segment.chars().all(|ch| ch.is_ascii_alphabetic()) {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            return first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase();
        }
    }

    if segment.len() == 2 && segment.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return segment.to_ascii_uppercase();
    }

    segment.to_string()
}

fn display_path(path: &Path) -> String {
    let raw = path.display().to_string();
    if cfg!(windows) {
        raw.trim_start_matches(r"\\?\").to_string()
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn normalizes_locale_tags() {
        let messages = Messages::new("zh_cn.UTF-8");
        assert_eq!(messages.locale(), "zh-CN");
    }

    #[test]
    fn falls_back_to_english_for_unknown_locale() {
        let messages = Messages::new("fr-FR");
        let rendered = messages.build_started(Path::new("repo"), Path::new("state"));
        assert_eq!(rendered, "Starting build for repo repo -> state state");
    }

    #[test]
    fn loads_translations_from_locale_files() {
        let messages = Messages::new("zh-TW");
        let rendered = messages.no_semantic_changes();
        assert_eq!(rendered, "未偵測到語意變更，只刷新狀態庫");
    }
}
