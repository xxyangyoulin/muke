use crate::{
    api::App,
    error::{Error, Result},
    tasks,
};
use anyhow::{Context, bail};
use serde_json::{Value, json};
use sqlx::FromRow;
use std::{sync::LazyLock, time::Duration};
use tokio::sync::Semaphore;

static SLOTS: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));

#[derive(FromRow)]
struct Profile {
    id: i64,
    name: String,
    protocol: String,
    base_url: String,
    api_key: String,
    model: String,
    max_tokens: i64,
}

pub async fn list(app: &App) -> Result<Value> {
    let profiles = sqlx::query_as::<_, Profile>("SELECT * FROM llm_profiles ORDER BY id")
        .fetch_all(&app.db)
        .await?;
    let calls = sqlx::query("SELECT id,profile_id,plugin,task_id,model,status,input_tokens,output_tokens,created_at FROM llm_calls ORDER BY id DESC LIMIT 50")
        .fetch_all(&app.db).await?.into_iter().map(crate::api::row_json).collect::<Vec<_>>();
    Ok(
        json!({"profiles":profiles.into_iter().map(|p| json!({"id":p.id,"name":p.name,"protocol":p.protocol,"base_url":p.base_url,"model":p.model,"max_tokens":p.max_tokens,"has_key":!p.api_key.is_empty()})).collect::<Vec<_>>(),"calls":calls}),
    )
}

pub async fn save(app: &App, input: &Value) -> Result<Value> {
    let text = |key: &str, limit| -> Result<String> {
        let value = input[key].as_str().unwrap_or("").trim();
        if value.is_empty() || value.len() > limit || value.contains(['\r', '\n']) {
            return Err(Error::invalid(format!("{key} 无效")));
        }
        Ok(value.into())
    };
    let name = text("name", 120)?;
    let protocol = text("protocol", 30)?;
    if !matches!(
        protocol.as_str(),
        "responses" | "chat_completions" | "anthropic"
    ) {
        return Err(Error::invalid("模型协议无效"));
    }
    let base_url = text("base_url", 1000)?.trim_end_matches('/').to_owned();
    let url = crate::network::parse_url(&base_url).map_err(|e| Error::invalid(e.to_string()))?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err(Error::invalid("模型地址不能包含查询参数或片段"));
    }
    let model = text("model", 200)?;
    let max_tokens = input["max_tokens"].as_i64().unwrap_or(2048);
    if !(256..=16384).contains(&max_tokens) {
        return Err(Error::invalid("输出上限应为 256–16384 tokens"));
    }
    let id = input["id"].as_i64();
    let _guard = app.content_lock.write().await;
    let previous = if let Some(id) = id {
        Some(
            sqlx::query_as::<_, Profile>("SELECT * FROM llm_profiles WHERE id=?")
                .bind(id)
                .fetch_optional(&app.db)
                .await?
                .ok_or_else(|| Error::not_found("模型配置不存在"))?,
        )
    } else {
        None
    };
    let key = input["api_key"].as_str().unwrap_or("").trim();
    if key.len() > 4000 || key.contains(['\r', '\n']) {
        return Err(Error::invalid("API Key 无效"));
    }
    let key = if key.is_empty() {
        if let Some(old) = &previous {
            if old.base_url != base_url {
                return Err(Error::invalid("修改接口地址时请重新填写 API Key"));
            }
            old.api_key.clone()
        } else {
            return Err(Error::invalid("请填写 API Key"));
        }
    } else {
        key.to_owned()
    };
    let id: i64 = sqlx::query_scalar("INSERT INTO llm_profiles(id,name,protocol,base_url,api_key,model,max_tokens) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,protocol=excluded.protocol,base_url=excluded.base_url,api_key=excluded.api_key,model=excluded.model,max_tokens=excluded.max_tokens RETURNING id")
        .bind(id).bind(name).bind(protocol).bind(base_url).bind(key).bind(model).bind(max_tokens).fetch_one(&app.db).await?;
    Ok(json!({"ok":true,"id":id}))
}

pub async fn remove(app: &App, id: i64) -> Result<Value> {
    let _guard = app.content_lock.write().await;
    sqlx::query("DELETE FROM llm_profiles WHERE id=?")
        .bind(id)
        .execute(&app.db)
        .await?;
    Ok(json!({"ok":true}))
}

fn payload(
    profile: &Profile,
    instructions: &str,
    input: &str,
) -> (String, Vec<(String, String)>, Value) {
    let mut headers = vec![("Content-Type".into(), "application/json".into())];
    let (path, body) = match profile.protocol.as_str() {
        "responses" => (
            "responses",
            json!({"model":profile.model,"instructions":instructions,"input":input,"max_output_tokens":profile.max_tokens,"store":false}),
        ),
        "chat_completions" => (
            "chat/completions",
            json!({"model":profile.model,"messages":[{"role":"system","content":instructions},{"role":"user","content":input}],"max_completion_tokens":profile.max_tokens}),
        ),
        "anthropic" => {
            headers.push(("x-api-key".into(), profile.api_key.clone()));
            headers.push(("anthropic-version".into(), "2023-06-01".into()));
            (
                "messages",
                json!({"model":profile.model,"system":instructions,"messages":[{"role":"user","content":input}],"max_tokens":profile.max_tokens}),
            )
        }
        _ => unreachable!(),
    };
    if profile.protocol != "anthropic" {
        headers.push((
            "Authorization".into(),
            format!("Bearer {}", profile.api_key),
        ));
    }
    (format!("{}/{path}", profile.base_url), headers, body)
}

fn response(protocol: &str, value: &Value) -> anyhow::Result<Value> {
    let (text, input, output) = match protocol {
        "responses" => {
            if value["status"] != "completed" {
                bail!("模型未完成输出，请调整输出上限或输入长度");
            }
            let text = value["output"]
                .as_array()
                .context("模型返回格式无效")?
                .iter()
                .filter(|item| item["type"] == "message")
                .flat_map(|item| item["content"].as_array().into_iter().flatten())
                .filter(|part| part["type"] == "output_text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            (
                text,
                &value["usage"]["input_tokens"],
                &value["usage"]["output_tokens"],
            )
        }
        "chat_completions" => {
            if value["choices"][0]["finish_reason"] != "stop" {
                bail!("模型未完成文本输出，请检查输出上限");
            }
            (
                value["choices"][0]["message"]["content"]
                    .as_str()
                    .context("模型没有返回文本")?
                    .into(),
                &value["usage"]["prompt_tokens"],
                &value["usage"]["completion_tokens"],
            )
        }
        "anthropic" => {
            match value["stop_reason"].as_str() {
                Some("end_turn" | "stop_sequence") => {}
                Some("max_tokens") => bail!(
                    "模型达到输出上限（stop_reason=max_tokens），请在后台模型配置中提高输出上限后重试"
                ),
                Some(reason) => bail!("模型未完成文本输出（stop_reason={reason}）"),
                None => bail!("模型响应缺少 stop_reason，请检查接口的 Anthropic 协议兼容性"),
            }
            let text = value["content"]
                .as_array()
                .context("模型返回格式无效")?
                .iter()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            (
                text,
                &value["usage"]["input_tokens"],
                &value["usage"]["output_tokens"],
            )
        }
        _ => unreachable!(),
    };
    if text.trim().is_empty() {
        bail!("模型返回空文本");
    }
    Ok(json!({"text":text,"usage":{"input_tokens":input,"output_tokens":output}}))
}

pub async fn generate(
    app: &App,
    ctx: &tasks::Context,
    plugin: &str,
    params: &Value,
) -> anyhow::Result<Value> {
    let id = params["profile"].as_i64().context("请选择模型配置")?;
    let instructions = params["instructions"].as_str().context("缺少分析提示词")?;
    let input = params["input"].as_str().context("缺少分析材料")?;
    if instructions.len() > 20_000 || input.len() > 200_000 || input.trim().is_empty() {
        bail!("模型输入为空或超出大小限制");
    }
    let profile = sqlx::query_as::<_, Profile>("SELECT * FROM llm_profiles WHERE id=?")
        .bind(id)
        .fetch_optional(&app.db)
        .await?
        .context("模型配置不存在，请重新选择")?;
    let _slot =
        tokio::select! { _=ctx.stop.cancelled()=>bail!("任务已取消"), slot=SLOTS.acquire()=>slot? };
    ctx.check()?;
    let call_id: i64 = {
        let _guard = tokio::select! { _=ctx.stop.cancelled()=>bail!("任务已取消"), guard=app.content_lock.write()=>guard };
        sqlx::query_scalar("INSERT INTO llm_calls(profile_id,plugin,task_id,model,status) VALUES(?,?,?,?,'running') RETURNING id").bind(id).bind(plugin).bind(ctx.id).bind(&profile.model).fetch_one(&app.db).await?
    };
    ctx.log(&format!("模型调用：{} / {}\n", profile.name, profile.model))
        .await?;
    let result = async {
        let (address,headers,body) = payload(&profile,instructions,input);
        for attempt in 0..3 {
            let (status,bytes) = app.network.request(&address,"POST",&headers,Some(serde_json::to_vec(&body)?),None,Duration::from_secs(120),true,2*1024*1024,&ctx.stop).await?;
            if (status == 429 || (500..600).contains(&status)) && attempt < 2 {
                tokio::select! { _=ctx.stop.cancelled()=>bail!("任务已取消"), _=tokio::time::sleep(Duration::from_secs(2 << attempt))=>{} }
                continue;
            }
            if !(200..300).contains(&status) { bail!("模型服务返回 HTTP {status}，请检查配置或额度"); }
            let value: Value = serde_json::from_slice(&bytes).context("模型服务没有返回 JSON")?;
            let mut result = response(&profile.protocol,&value)?;
            result["model"] = json!(profile.model);
            return Ok(result);
        }
        unreachable!()
    }.await;
    let status = if result.is_ok() {
        "succeeded"
    } else if ctx.stop.is_cancelled() {
        "cancelled"
    } else {
        "failed"
    };
    let usage = result.as_ref().ok().map(|value: &Value| &value["usage"]);
    let _guard = app.content_lock.write().await;
    sqlx::query("UPDATE llm_calls SET status=?,input_tokens=?,output_tokens=? WHERE id=?")
        .bind(status)
        .bind(usage.and_then(|value| value["input_tokens"].as_i64()))
        .bind(usage.and_then(|value| value["output_tokens"].as_i64()))
        .bind(call_id)
        .execute(&app.db)
        .await?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anthropic_incomplete_reasons_are_distinguished() {
        let truncated = response(
            "anthropic",
            &json!({"stop_reason":"max_tokens","content":[{"type":"text","text":"partial"}]}),
        )
        .unwrap_err()
        .to_string();
        assert!(truncated.contains("stop_reason=max_tokens"));
        assert!(truncated.contains("提高输出上限"));
        let refusal = response("anthropic", &json!({"stop_reason":"refusal"}))
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("stop_reason=refusal"));
        assert!(!refusal.contains("输出上限"));
        assert!(
            response("anthropic", &json!({"content":[]}))
                .unwrap_err()
                .to_string()
                .contains("缺少 stop_reason")
        );
    }

    #[test]
    fn provider_payloads_and_completion_validation() {
        let mut profile = Profile {
            id: 1,
            name: "test".into(),
            protocol: "responses".into(),
            base_url: "https://example.org/v1".into(),
            api_key: "private".into(),
            model: "chosen-model".into(),
            max_tokens: 2048,
        };
        let (url, headers, body) = payload(&profile, "instructions", "evidence");
        assert_eq!(url, "https://example.org/v1/responses");
        assert_eq!(body["store"], false);
        assert!(headers.iter().any(|(name, _)| name == "Authorization"));
        assert_eq!(response("responses",&json!({"status":"completed","output":[{"type":"reasoning","summary":[]},{"type":"message","content":[{"type":"output_text","text":"analysis"}]}],"usage":{"input_tokens":12,"output_tokens":5}})).unwrap()["text"],"analysis");
        assert!(response("responses", &json!({"status":"incomplete","output":[]})).is_err());
        profile.protocol = "chat_completions".into();
        assert_eq!(
            payload(&profile, "instructions", "input").2["messages"][0]["role"],
            "system"
        );
        assert_eq!(response("chat_completions",&json!({"choices":[{"finish_reason":"stop","message":{"content":"ok"}}],"usage":{"prompt_tokens":9,"completion_tokens":2}})).unwrap()["usage"]["input_tokens"],9);
        assert!(
            response(
                "chat_completions",
                &json!({"choices":[{"finish_reason":"length","message":{"content":"cut"}}]})
            )
            .is_err()
        );
        profile.protocol = "anthropic".into();
        let (_, headers, body) = payload(&profile, "instructions", "input");
        assert!(headers.iter().any(|(name, _)| name == "x-api-key"));
        assert!(!headers.iter().any(|(name, _)| name == "Authorization"));
        assert_eq!(body["system"], "instructions");
        assert_eq!(response("anthropic",&json!({"stop_reason":"end_turn","content":[{"type":"text","text":"one"},{"type":"text","text":"two"}],"usage":{"input_tokens":3,"output_tokens":2}})).unwrap()["text"],"one\ntwo");
    }
}
