use serde_json::{Value, json};
use std::io::{self, BufRead};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let request: Value = serde_json::from_str(&line)?;
    if request["protocol"] != 1 {
        println!("{}", json!({"type":"error","message":"不支持的协议版本"}));
        return Ok(());
    }
    let message = request["config"]["message"]
        .as_str()
        .unwrap_or("你好，MUKE！");
    let response = match request["method"].as_str() {
        Some("check") => {
            json!({"result":{"protocol":1,"id":"hello","version":env!("CARGO_PKG_VERSION")}})
        }
        Some("sync") => {
            println!(
                "{}",
                json!({"type":"progress","phase":"发布问候动态","completed":0,"total":1})
            );
            eprintln!("开始同步独立插件");
            json!({"result":{"message":message},"state":{"last_message":message},"posts":[{"key":"welcome","content":message}]})
        }
        Some("data") => json!({"result":{"message":message}}),
        Some("admin") => json!({"result":{"ok":true},"state":request["params"]["state"]}),
        Some("export") => {
            let escaped = message
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&#39;");
            json!({"result":{"items":{"message":message,"html":format!("<section><h1>独立插件</h1><p>{escaped}</p></section>")},"meta":{}}})
        }
        _ => {
            println!("{}", json!({"type":"error","message":"不支持的方法"}));
            return Ok(());
        }
    };
    let mut response = response;
    response["type"] = json!("result");
    println!("{response}");
    Ok(())
}
