//! Hub HTTP 客户端（blocking reqwest）。把开放 API 封装为方法，错误带状态码。

use std::time::Duration;

use crate::shared::{
    AuthReq, AuthResp, McpInfo, McpManifest, McpUpsertReq, ReportCallReq, ResolveResp, SecretInfo,
    SecretSetReq, SetToolsReq, SkillInfo, SkillManifest, SkillUpsertReq, SkillVersionInfo, ToolMeta,
};

/// 带 HTTP 状态码的客户端错误，status==0 表示传输层失败。
#[derive(Debug)]
pub struct HubError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.status == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "[{}] {}", self.status, self.message)
        }
    }
}
impl std::error::Error for HubError {}

impl HubError {
    fn transport(e: impl std::fmt::Display) -> Self {
        HubError {
            status: 0,
            message: format!("无法连接 Hub: {e}"),
        }
    }
}

pub struct HubClient {
    base: String,
    token: Option<String>,
    http: reqwest::blocking::Client,
}

impl HubClient {
    pub fn new(base: impl Into<String>, token: Option<String>) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("构建 HTTP 客户端");
        HubClient {
            base: base.into().trim_end_matches('/').to_string(),
            token,
            http,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn auth(&self, rb: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
        match &self.token {
            Some(t) => rb.bearer_auth(t),
            None => rb,
        }
    }

    fn parse<T: serde::de::DeserializeOwned>(
        resp: reqwest::blocking::Response,
    ) -> Result<T, HubError> {
        let status = resp.status();
        if status.is_success() {
            resp.json::<T>().map_err(|e| HubError {
                status: status.as_u16(),
                message: format!("解析响应失败: {e}"),
            })
        } else {
            let code = status.as_u16();
            let message = resp
                .json::<crate::shared::ErrorResp>()
                .map(|e| e.error)
                .unwrap_or_else(|_| status.to_string());
            Err(HubError { status: code, message })
        }
    }

    fn ok_empty(resp: reqwest::blocking::Response) -> Result<(), HubError> {
        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            let code = status.as_u16();
            let message = resp
                .json::<crate::shared::ErrorResp>()
                .map(|e| e.error)
                .unwrap_or_else(|_| status.to_string());
            Err(HubError { status: code, message })
        }
    }

    pub fn register(&self, username: &str, password: &str) -> Result<AuthResp, HubError> {
        let resp = self
            .http
            .post(self.url("/v1/auth/register"))
            .json(&AuthReq {
                username: username.into(),
                password: password.into(),
            })
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    pub fn login(&self, username: &str, password: &str) -> Result<AuthResp, HubError> {
        let resp = self
            .http
            .post(self.url("/v1/auth/login"))
            .json(&AuthReq {
                username: username.into(),
                password: password.into(),
            })
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    pub fn mcp_upsert(&self, manifest: McpManifest, visibility: String) -> Result<McpInfo, HubError> {
        let resp = self
            .auth(self.http.post(self.url("/v1/mcp")))
            .json(&McpUpsertReq { manifest, visibility })
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    pub fn mcp_list(&self) -> Result<Vec<McpInfo>, HubError> {
        let resp = self
            .auth(self.http.get(self.url("/v1/mcp")))
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    /// 搜索 Hub 中的公开 MCP（名称/描述关键字匹配）。无需登录。
    pub fn explore(&self, query: &str) -> Result<Vec<McpInfo>, HubError> {
        let mut rb = self.http.get(self.url("/v1/explore"));
        if !query.is_empty() {
            rb = rb.query(&[("q", query)]);
        }
        let resp = rb.send().map_err(HubError::transport)?;
        Self::parse(resp)
    }

    pub fn mcp_delete(&self, name: &str) -> Result<(), HubError> {
        let resp = self
            .auth(self.http.delete(self.url(&format!("/v1/mcp/{name}"))))
            .send()
            .map_err(HubError::transport)?;
        Self::ok_empty(resp)
    }

    pub fn secret_set(&self, key: &str, value: &str) -> Result<SecretInfo, HubError> {
        let resp = self
            .auth(self.http.put(self.url("/v1/secret")))
            .json(&SecretSetReq {
                key: key.into(),
                value: value.into(),
            })
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    pub fn secret_list(&self) -> Result<Vec<SecretInfo>, HubError> {
        let resp = self
            .auth(self.http.get(self.url("/v1/secret")))
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    pub fn secret_delete(&self, key: &str) -> Result<(), HubError> {
        let resp = self
            .auth(self.http.delete(self.url(&format!("/v1/secret/{key}"))))
            .send()
            .map_err(HubError::transport)?;
        Self::ok_empty(resp)
    }

    pub fn run_resolve(&self, owner: &str, name: &str) -> Result<ResolveResp, HubError> {
        let resp = self
            .auth(
                self.http
                    .post(self.url(&format!("/v1/run/{owner}/{name}/resolve"))),
            )
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    /// 回传一次本地 `tsk run` 工具调用的审计元信息（尽力而为，仅记录成败/耗时/结果摘要）。
    pub fn report_call(
        &self,
        owner: &str,
        name: &str,
        tool: &str,
        ok: bool,
        error: &str,
        ms: i64,
        summary: &str,
    ) -> Result<(), HubError> {
        let resp = self
            .auth(
                self.http
                    .post(self.url(&format!("/v1/run/{owner}/{name}/report"))),
            )
            .json(&ReportCallReq {
                tool: tool.to_string(),
                ok,
                error: error.to_string(),
                ms,
                summary: summary.to_string(),
            })
            .send()
            .map_err(HubError::transport)?;
        Self::ok_empty(resp)
    }

    /// 上报某 MCP 的工具清单（写入检索索引；仅限本人）。返回已索引数量。
    pub fn set_tools(&self, name: &str, tools: &[ToolMeta]) -> Result<usize, HubError> {
        let resp = self
            .auth(self.http.post(self.url(&format!("/v1/mcp/{name}/tools"))))
            .json(&SetToolsReq {
                tools: tools.to_vec(),
            })
            .send()
            .map_err(HubError::transport)?;
        let v: serde_json::Value = Self::parse(resp)?;
        Ok(v.get("indexed").and_then(|x| x.as_u64()).unwrap_or(0) as usize)
    }

    // -----------------------------------------------------------------------
    // 技能市场
    // -----------------------------------------------------------------------

    /// 搜索公开技能（按关键字 / 分类 / 标签）。无需登录。
    pub fn skill_explore(
        &self,
        query: &str,
        category: Option<&str>,
        tag: Option<&str>,
    ) -> Result<Vec<SkillInfo>, HubError> {
        let mut rb = self.http.get(self.url("/v1/skill/explore"));
        let mut q: Vec<(&str, &str)> = Vec::new();
        if !query.is_empty() {
            q.push(("q", query));
        }
        if let Some(c) = category.filter(|s| !s.is_empty()) {
            q.push(("category", c));
        }
        if let Some(t) = tag.filter(|s| !s.is_empty()) {
            q.push(("tag", t));
        }
        if !q.is_empty() {
            rb = rb.query(&q);
        }
        let resp = rb.send().map_err(HubError::transport)?;
        Self::parse(resp)
    }

    /// 列出名下全部技能（含私有）。
    pub fn skill_list(&self) -> Result<Vec<SkillInfo>, HubError> {
        let resp = self
            .auth(self.http.get(self.url("/v1/skill")))
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    /// 技能详情（public 任何人可读；private 需 owner token）。
    /// `version` 指定历史版本（None 为最新版），响应的 `versions` 列出全部可用版本。
    pub fn skill_get(
        &self,
        owner: &str,
        name: &str,
        version: Option<&str>,
    ) -> Result<SkillInfo, HubError> {
        let mut rb = self.auth(self.http.get(self.url(&format!("/v1/skill/{owner}/{name}"))));
        if let Some(v) = version.filter(|v| !v.is_empty()) {
            rb = rb.query(&[("version", v)]);
        }
        let resp = rb.send().map_err(HubError::transport)?;
        Self::parse(resp)
    }

    /// 列出技能已发布的全部版本副本（新→旧）。
    pub fn skill_versions(&self, owner: &str, name: &str) -> Result<Vec<SkillVersionInfo>, HubError> {
        let resp = self
            .auth(self.http.get(self.url(&format!("/v1/skill/{owner}/{name}/versions"))))
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    /// 发布/更新技能元数据（压缩体单独上传）。
    pub fn skill_upsert(
        &self,
        manifest: &SkillManifest,
        visibility: &str,
        skill_md: &str,
        archive_sha256: &str,
        archive_size: u64,
    ) -> Result<SkillInfo, HubError> {
        let resp = self
            .auth(self.http.post(self.url("/v1/skill")))
            .json(&SkillUpsertReq {
                manifest: manifest.clone(),
                visibility: visibility.to_string(),
                skill_md: skill_md.to_string(),
                archive_sha256: archive_sha256.to_string(),
                archive_size,
            })
            .send()
            .map_err(HubError::transport)?;
        Self::parse(resp)
    }

    /// 上传技能压缩体（tar.zst 原始字节）。`version` 指定挂到哪个版本副本（None 为最新版）。
    pub fn skill_archive_put(
        &self,
        owner: &str,
        name: &str,
        version: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<(), HubError> {
        let mut rb = self
            .auth(self.http.put(self.url(&format!("/v1/skill/{owner}/{name}/archive"))))
            .header(reqwest::header::CONTENT_TYPE, "application/zstd");
        if let Some(v) = version.filter(|v| !v.is_empty()) {
            rb = rb.query(&[("version", v)]);
        }
        let resp = rb.body(bytes).send().map_err(HubError::transport)?;
        let _: serde_json::Value = Self::parse(resp)?;
        Ok(())
    }

    /// 下载技能压缩体（tar.zst 原始字节，旧包可能仍为 gzip）。`version` 指定历史版本（None 为最新版）。
    pub fn skill_archive_get(
        &self,
        owner: &str,
        name: &str,
        version: Option<&str>,
    ) -> Result<Vec<u8>, HubError> {
        let mut rb = self.auth(self.http.get(self.url(&format!("/v1/skill/{owner}/{name}/archive"))));
        if let Some(v) = version.filter(|v| !v.is_empty()) {
            rb = rb.query(&[("version", v)]);
        }
        let resp = rb.send().map_err(HubError::transport)?;
        let status = resp.status();
        if !status.is_success() {
            let code = status.as_u16();
            let message = resp
                .json::<crate::shared::ErrorResp>()
                .map(|e| e.error)
                .unwrap_or_else(|_| status.to_string());
            return Err(HubError { status: code, message });
        }
        resp.bytes()
            .map(|b| b.to_vec())
            .map_err(|e| HubError {
                status: status.as_u16(),
                message: format!("读取压缩体失败: {e}"),
            })
    }

    pub fn skill_delete(&self, owner: &str, name: &str) -> Result<(), HubError> {
        let resp = self
            .auth(self.http.delete(self.url(&format!("/v1/skill/{owner}/{name}"))))
            .send()
            .map_err(HubError::transport)?;
        Self::ok_empty(resp)
    }
}
