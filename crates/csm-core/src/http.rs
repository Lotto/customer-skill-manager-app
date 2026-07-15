//! Real HTTP [`SkillSource`] backed by the CSM backend.
//!
//! Compiled only with the `net` feature so the pure-logic tests stay offline.

use crate::backoff::backoff_delays;
use crate::error::{CoreError, Result};
use crate::manifest::{parse_skill_list, SkillManifest};
use crate::sync::SkillSource;
use std::time::Duration;

/// User-Agent the backend expects (kept in sync with the original loader).
pub const USER_AGENT: &str = "csm-loader/1.0.0";

/// The `Accept` header advertised for instruction/resource requests.
const ACCEPT: &str = "text/markdown, text/plain, application/json";

/// HTTP skill source. One instance per (endpoint, license) pair.
pub struct HttpSkillSource {
    client: reqwest::blocking::Client,
    endpoint: String,
    license_key: String,
    retries: u32,
}

impl HttpSkillSource {
    /// Build a source. `timeout` bounds each individual request.
    pub fn new(
        endpoint: impl Into<String>,
        license_key: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .build()
            .map_err(|e| CoreError::Http(e.to_string()))?;
        Ok(Self {
            client,
            endpoint: endpoint.into(),
            license_key: license_key.into(),
            retries: 4,
        })
    }

    /// GET with retry/backoff for transient failures. Permanent failures
    /// (license, 404) short-circuit immediately.
    fn get(&self, query: &[(&str, &str)]) -> Result<String> {
        let delays = backoff_delays(
            self.retries,
            Duration::from_millis(500),
            Duration::from_secs(10),
        );
        let mut attempt = 0usize;
        loop {
            match self.try_get(query) {
                Ok(body) => return Ok(body),
                Err(e) if e.is_permanent() || attempt >= delays.len() => return Err(e),
                Err(_) => {
                    std::thread::sleep(delays[attempt]);
                    attempt += 1;
                }
            }
        }
    }

    fn try_get(&self, query: &[(&str, &str)]) -> Result<String> {
        let resp = self
            .client
            .get(&self.endpoint)
            .header("X-License-Key", &self.license_key)
            .header("Accept", ACCEPT)
            .query(query)
            .send()
            .map_err(|e| CoreError::Http(e.to_string()))?;

        let status = resp.status().as_u16();
        match status {
            200 => resp.text().map_err(|e| CoreError::Http(e.to_string())),
            402 => Err(CoreError::SubscriptionInactive),
            403 => Err(CoreError::LicenseInvalid),
            404 => Err(CoreError::NotFound(format!("{query:?}"))),
            429 => Err(CoreError::RateLimited),
            500..=599 => Err(CoreError::ServerError(status)),
            other => Err(CoreError::Http(format!("unexpected status {other}"))),
        }
    }
}

impl SkillSource for HttpSkillSource {
    fn fetch_manifest(&self) -> Result<SkillManifest> {
        let body = self.get(&[("resource", "__list")])?;
        Ok(parse_skill_list(&body))
    }

    fn fetch_instructions(&self, slug: &str) -> Result<String> {
        let body = self.get(&[("slug", slug), ("resource", "instructions")])?;
        Ok(strip_leading_html_comment(&body).trim().to_string())
    }
}

/// Strip a leading `<!-- ... -->` metadata block (the backend watermark) so the
/// materialized `SKILL.md` is clean and its content hash is stable across
/// fetches that only differ by the `csm:served` timestamp.
pub fn strip_leading_html_comment(s: &str) -> &str {
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix("<!--") {
        if let Some(end) = rest.find("-->") {
            return rest[end + 3..].trim_start();
        }
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_leading_comment() {
        let body =
            "<!--\ncsm:served 2026-07-15\ncsm:skill \"bonjour@1.0.0\"\n-->\n# Contexte\n\nbody";
        assert_eq!(strip_leading_html_comment(body), "# Contexte\n\nbody");
    }

    #[test]
    fn leaves_plain_content_untouched() {
        assert_eq!(strip_leading_html_comment("# Title\n\nx"), "# Title\n\nx");
    }

    #[test]
    fn handles_unterminated_comment_gracefully() {
        // No closing marker: return the trimmed original rather than panicking.
        let s = "<!-- oops no end";
        assert_eq!(strip_leading_html_comment(s), s);
    }
}
