use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

/// 静态导出专用迷你打包器：把 registry.js 与 app.js 的原生 ES 模块依赖图拼接为
/// 单个经典脚本，减少静态产物的请求文件数。视图层存在循环引用（ES 活绑定语义），
/// 因此采用单作用域拼接：按依赖序去掉 import 语句与 export 前缀直接合并；
/// 顶层声明重名时放弃打包（保留模块化复制）。动态服务继续使用模块化前端。
/// 待打包的插件模块：id 用于注册已打包标记，destination 为产物内待删除的模块文件。
pub struct PluginModule {
    pub id: String,
    pub source: PathBuf,
    pub destination: PathBuf,
}

pub fn bundle(assets: &Path, destination: &Path, plugins: &[PluginModule]) -> Result<()> {
    let entry = "app.js";
    let mut order = Vec::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    visit(assets, entry, &mut visited, &mut order, &mut sources)?;
    let registry = std::fs::read_to_string(assets.join("registry.js"))?;
    // 入口模块最后执行：插件模块插到入口之前（其注册副作用须先于应用启动）
    let entry = order.pop().context("缺少入口模块 app.js")?;
    let mut plugin_modules = Vec::new();
    for plugin in plugins {
        let source = std::fs::read_to_string(&plugin.source)
            .with_context(|| format!("缺少插件前端模块：{}", plugin.source.display()))?;
        plugin_modules.push((plugin, source));
    }
    let mut declared: HashMap<String, String> = HashMap::new();
    let mut output = String::new();
    output.push_str("// 由静态导出打包生成：registry + ES 模块图按依赖序拼接为单文件\n");
    output.push_str("(function () {\n'use strict';\n");
    output.push_str(&registry);
    // 模块代码依赖完整 DOM（原为 type=module 的延迟语义），就绪后再执行；
    // registry 部分同步执行，保留 has-js 标记先于首帧绘制的防闪动行为
    output.push_str("\nfunction __start() {\n");
    for (plugin, source) in &plugin_modules {
        let transformed = transform(source)?;
        sources.insert(plugin.id.clone(), transformed);
        order.push(plugin.id.clone());
    }
    if !plugin_modules.is_empty() {
        let ids: Vec<String> = plugin_modules
            .iter()
            .map(|(plugin, _)| format!("{}: true", plugin.id))
            .collect();
        output.push_str(&format!(
            "\nwindow.MKBLOG._bundled = {{ {} }};\n",
            ids.join(", ")
        ));
    }
    order.push(entry);
    let plugin_ids: BTreeSet<&str> = plugin_modules
        .iter()
        .map(|(plugin, _)| plugin.id.as_str())
        .collect();
    for id in &order {
        let transformed = transform(&sources[id])?;
        if plugin_ids.contains(id.as_str()) {
            // 插件模块各有内部顶层名（如 ID），包进独立 IIFE 隔离；
            // 对核心作用域名（store、api 等）的引用经外层作用域解析
            output.push_str(&format!(
                "\n// ---- {id} ----\n(() => {{\n{}\n}})();\n",
                transformed
            ));
            continue;
        }
        for name in top_level_names(&transformed) {
            if let Some(previous) = declared.insert(name.clone(), id.clone()) {
                bail!("顶层声明重名：{name}（{previous} 与 {id}）");
            }
        }
        output.push_str(&format!("\n// ---- {id} ----\n"));
        output.push_str(&transformed);
        output.push('\n');
    }
    output.push_str(
        "\n}\nif (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', __start); else __start();\n})();\n",
    );
    std::fs::write(destination.join("app.js"), output)?;

    let mut consumed: BTreeSet<PathBuf> = order.iter().map(PathBuf::from).collect();
    consumed.insert("registry.js".into());
    for id in &consumed {
        if id.as_os_str() != "app.js" {
            let path = destination.join(id);
            if path.is_file() {
                std::fs::remove_file(path)?;
            }
        }
    }
    for (plugin, _) in &plugin_modules {
        if plugin.destination.is_file() {
            std::fs::remove_file(&plugin.destination)?;
        }
    }
    Ok(())
}

fn visit(
    assets: &Path,
    id: &str,
    visited: &mut BTreeSet<String>,
    order: &mut Vec<String>,
    sources: &mut HashMap<String, String>,
) -> Result<()> {
    if !visited.insert(id.to_owned()) {
        return Ok(());
    }
    let source =
        std::fs::read_to_string(assets.join(id)).with_context(|| format!("缺少前端模块：{id}"))?;
    for specifier in import_specifiers(&source) {
        let child = resolve(id, &specifier)?;
        if !visited.contains(&child) {
            visit(assets, &child, visited, order, sources)?;
        }
    }
    order.push(id.to_owned());
    sources.insert(id.to_owned(), source);
    Ok(())
}

fn import_specifiers(source: &str) -> Vec<String> {
    static IMPORT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(?s)import\s*\{[^}]+\}\s*from\s*['"](\.[^'"]+)['"];?"#).unwrap()
    });
    IMPORT
        .captures_iter(source)
        .map(|captures| captures[1].to_owned())
        .collect()
}

/// 去掉 import 语句与 export 前缀（依赖名由共享作用域提供）。
/// 别名导入（`a as b`）无法靠共享作用域表达，遇到即放弃打包。
fn transform(source: &str) -> Result<String> {
    static ALIASED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?s)import\s*\{[^}]*\bas\b[^}]*\}\s*from").unwrap()
    });
    if ALIASED.is_match(source) {
        bail!("包含别名导入，无法单作用域拼接");
    }
    static IMPORT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?s)import\s*\{[^}]+\}\s*from\s*['"](\.[^'"]+|/assets/[^'"]+)['"];?[\t ]*(?://[^\n]*)?\n?"#,
        )
        .unwrap()
    });
    static FUNCTION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?m)^export (async function|function|const|let|class) ").unwrap()
    });
    let stripped = IMPORT.replace_all(source, "").into_owned();
    Ok(FUNCTION.replace_all(&stripped, "$1 ").into_owned())
}

/// 收集行首声明名用于重名检测（function/const/let/class/var）。
fn top_level_names(source: &str) -> Vec<String> {
    static DECLARATION: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?m)^(?:async function|function|const|let|class|var)\s+([A-Za-z_$][\w$]*)",
        )
        .unwrap()
    });
    DECLARATION
        .captures_iter(source)
        .map(|captures| captures[1].to_owned())
        .collect()
}

/// 相对导入解析为相对 assets 根的模块 id（正斜杠，规范化 ./ 与 ../）。
fn resolve(from: &str, specifier: &str) -> Result<String> {
    let joined = match from.rsplit_once('/') {
        Some((directory, _)) => format!("{directory}/{specifier}"),
        None => specifier.to_owned(),
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                if parts.pop().is_none() {
                    bail!("无法解析导入：{specifier}");
                }
            }
            other => parts.push(other),
        }
    }
    let id = parts.join("/");
    if id.is_empty() {
        bail!("无法解析导入：{specifier}");
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concatenates_modules_in_dependency_order_and_keeps_standalone_assets() {
        let root = tempfile::tempdir().unwrap();
        let assets = root.path().join("assets");
        std::fs::create_dir_all(assets.join("lib")).unwrap();
        std::fs::create_dir_all(assets.join("views")).unwrap();
        std::fs::write(
            assets.join("registry.js"),
            "window.MKBLOG = { esc: v => v };",
        )
        .unwrap();
        // 循环依赖（活绑定语义）在单作用域拼接下天然可用
        std::fs::write(
            assets.join("lib/ui.js"),
            "import { admin } from '../views/admin.js';\nexport const esc = v => String(v);\nexport function icon(name) { return admin() + name; }",
        )
        .unwrap();
        std::fs::write(
            assets.join("views/admin.js"),
            "import { esc } from '../lib/ui.js';\nexport function admin() { return esc('a'); }",
        )
        .unwrap();
        std::fs::write(
            assets.join("app.js"),
            "import { esc } from './lib/ui.js';\nimport { admin } from './views/admin.js';\nboot();\nfunction boot() { return esc(admin()); }",
        )
        .unwrap();
        std::fs::write(assets.join("embed-init.js"), "standalone();").unwrap();
        std::fs::write(assets.join("style.css"), "body{}").unwrap();
        bundle(&assets, &assets, &[]).unwrap();
        let bundled = std::fs::read_to_string(assets.join("app.js")).unwrap();
        assert!(bundled.contains("window.MKBLOG"));
        assert!(bundled.contains("// ---- lib/ui.js ----"));
        assert!(bundled.contains("// ---- views/admin.js ----"));
        assert!(bundled.contains("// ---- app.js ----"));
        assert!(!bundled.contains("export "));
        assert!(!bundled.contains("import {"));
        assert!(!assets.join("registry.js").exists());
        assert!(!assets.join("lib").join("ui.js").exists());
        assert!(!assets.join("views").join("admin.js").exists());
        assert!(assets.join("embed-init.js").is_file());
        assert!(assets.join("style.css").is_file());
        // 插件模块拼接进单文件并删除产物内副本（重建被上次打包删除的文件）
        std::fs::write(
            assets.join("registry.js"),
            "window.MKBLOG = { registerCard: function () {} };",
        )
        .unwrap();
        std::fs::create_dir_all(assets.join("views")).unwrap();
        std::fs::create_dir_all(root.path().join("plugins/rss/frontend")).unwrap();
        std::fs::write(
            root.path().join("plugins/rss/frontend/plugin.js"),
            "import { store } from '/assets/lib/store.js';\nwindow.MKBLOG.registerCard('rss', () => store.isStatic);\nexport function unused() { return 1; }",
        )
        .unwrap();
        std::fs::create_dir_all(assets.join("lib")).unwrap();
        std::fs::write(
            assets.join("lib/ui.js"),
            "export const esc = v => String(v);",
        )
        .unwrap();
        std::fs::write(
            assets.join("views/admin.js"),
            "export function admin() { return 'a'; }",
        )
        .unwrap();
        std::fs::write(
            assets.join("app.js"),
            "import { esc } from './lib/ui.js';\nboot();\nfunction boot() { return esc('x'); }",
        )
        .unwrap();
        let plugin_dest = assets.join("__plugin_marker.js");
        bundle(
            &assets,
            &assets,
            &[PluginModule {
                id: "rss".to_owned(),
                source: root.path().join("plugins/rss/frontend/plugin.js"),
                destination: plugin_dest.clone(),
            }],
        )
        .unwrap();
        let bundled = std::fs::read_to_string(assets.join("app.js")).unwrap();
        assert!(bundled.contains("window.MKBLOG._bundled = { rss: true }"));
        assert!(bundled.contains("registerCard('rss'"));
        assert!(!bundled.contains("import { store }"));
        assert!(!plugin_dest.exists());
        // 插件内部顶层名重名不冲突（IIFE 隔离；重建核心文件）
        std::fs::write(
            assets.join("registry.js"),
            "window.MKBLOG = { registerCard: function () {} };",
        )
        .unwrap();
        std::fs::create_dir_all(assets.join("lib")).unwrap();
        std::fs::write(
            assets.join("lib/ui.js"),
            "export const esc = v => String(v);",
        )
        .unwrap();
        std::fs::create_dir_all(assets.join("views")).unwrap();
        std::fs::write(
            assets.join("views/admin.js"),
            "export function admin() { return 'a'; }",
        )
        .unwrap();
        std::fs::write(
            assets.join("app.js"),
            "import { esc } from './lib/ui.js';\nboot();\nfunction boot() { return esc('x'); }",
        )
        .unwrap();
        std::fs::create_dir_all(root.path().join("plugins/wallpapers/frontend")).unwrap();
        std::fs::write(
            root.path().join("plugins/wallpapers/frontend/plugin.js"),
            "const ID = 'wallpapers';\nwindow.MKBLOG.registerCard('wallpapers', () => ID);",
        )
        .unwrap();
        std::fs::write(
            root.path().join("plugins/rss/frontend/plugin.js"),
            "const ID = 'rss';\nwindow.MKBLOG.registerCard('rss', () => ID);",
        )
        .unwrap();
        bundle(
            &assets,
            &assets,
            &[
                PluginModule {
                    id: "rss".to_owned(),
                    source: root.path().join("plugins/rss/frontend/plugin.js"),
                    destination: assets.join("__rss.js"),
                },
                PluginModule {
                    id: "wallpapers".to_owned(),
                    source: root.path().join("plugins/wallpapers/frontend/plugin.js"),
                    destination: assets.join("__wallpapers.js"),
                },
            ],
        )
        .unwrap();
        let bundled = std::fs::read_to_string(assets.join("app.js")).unwrap();
        assert_eq!(bundled.matches("const ID =").count(), 2);
        assert!(bundled.contains("(() => {"));
        // 顶层重名：放弃打包（回退由调用方保留原样复制）
        std::fs::create_dir_all(assets.join("lib")).unwrap();
        std::fs::write(
            assets.join("lib/ui.js"),
            "export const esc = v => String(v);",
        )
        .unwrap();
        std::fs::write(
            assets.join("views/admin.js"),
            "export function admin() { return 'a'; }",
        )
        .unwrap();
        std::fs::write(
            assets.join("app.js"),
            "import { esc } from './lib/ui.js';\nconst esc = 1;",
        )
        .unwrap();
        assert!(bundle(&assets, &assets, &[]).is_err());
    }
}
