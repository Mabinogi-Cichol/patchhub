//! Responses captured by the host in a user-verified browser session.
//! Lua still makes the requests and parses the pages through RequestContext.
use super::{
    http::{FetchRequest, FetchResponse, Fetcher},
    invalid, Result,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
#[serde(deny_unknown_fields)]
pub struct BrowserResponse {
    pub url: String,
    pub method: String,
    pub body: Option<String>,
    pub response_url: String,
    pub status: u16,
    pub text: String,
}

pub struct BrowserFetcher(Mutex<VecDeque<BrowserResponse>>);

impl BrowserFetcher {
    pub fn new(responses: Vec<BrowserResponse>) -> Self {
        Self(Mutex::new(responses.into()))
    }
}

impl Fetcher for BrowserFetcher {
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse> {
        let page = self
            .0
            .lock()
            .map_err(|_| invalid("browser_session_failed"))?
            .pop_front()
            .ok_or_else(|| invalid("browser_response_missing"))?;
        if page.url != request.url
            || page.method != request.method
            || page.body.as_ref().map(|s| s.as_bytes()) != request.body.as_deref()
        {
            return Err(invalid("browser_response_request_mismatch"));
        }
        Ok(FetchResponse {
            status: page.status,
            body: page.text.into_bytes(),
            final_url: page.response_url,
            content_type: Some("text/html; charset=utf-8".into()),
            ..Default::default()
        })
    }
}
