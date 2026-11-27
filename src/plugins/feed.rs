use anyhow::{Result, bail};
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};
use std::{borrow::Cow, sync::LazyLock};

pub fn decode(bytes: &[u8]) -> Result<Cow<'_, str>> {
    static DECLARATION: LazyLock<regex::bytes::Regex> = LazyLock::new(|| {
        regex::bytes::Regex::new(r#"^<\?xml\s+[^?]*\bencoding\s*=\s*["']([^"']+)["']"#).unwrap()
    });
    let (encoding, skip) = if let Some(bom) = Encoding::for_bom(bytes) {
        bom
    } else if bytes.starts_with(b"\0<\0?") {
        (UTF_16BE, 0)
    } else if bytes.starts_with(b"<\0?\0") {
        (UTF_16LE, 0)
    } else if let Some(declaration) = DECLARATION.captures(bytes) {
        (
            Encoding::for_label(&declaration[1])
                .ok_or_else(|| anyhow::anyhow!("不支持的 XML 字符编码"))?,
            0,
        )
    } else {
        (UTF_8, 0)
    };
    let (text, errors) = encoding.decode_without_bom_handling(&bytes[skip..]);
    if errors {
        bail!("XML 字符编码无效或数据损坏");
    }
    Ok(text)
}

pub fn title(node: roxmltree::Node<'_, '_>) -> String {
    let Some(title) = node
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "title")
    else {
        return String::new();
    };
    let atom = title.tag_name().namespace() == Some("http://www.w3.org/2005/Atom");
    let value = match title.attribute("type").unwrap_or("text") {
        "xhtml" if atom => title
            .descendants()
            .filter(|n| n.is_text())
            .filter_map(|n| n.text())
            .collect::<String>(),
        "text" if atom => title.text().unwrap_or("").to_owned(),
        _ => scraper::Html::parse_fragment(title.text().unwrap_or(""))
            .root_element()
            .text()
            .collect::<String>(),
    };
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn timestamp(value: &str, timezone: chrono_tz::Tz) -> Option<i64> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime, Offset, TimeZone};
    if let Ok(date) =
        DateTime::parse_from_rfc2822(value).or_else(|_| DateTime::parse_from_rfc3339(value))
    {
        return Some(date.timestamp());
    }
    for format in ["%Y-%m-%d %H:%M:%S %z", "%Y-%m-%d %H:%M:%S%:z"] {
        if let Ok(date) = DateTime::parse_from_str(value, format) {
            return Some(date.timestamp());
        }
    }
    // 命名 UTC/GMT 时区后缀按 UTC 解释（与 PHP strtotime 一致）
    let zoneless = value.trim();
    let (zoneless, utc_named) = match zoneless
        .rsplit_once(' ')
        .filter(|(_, zone)| matches!(*zone, "UTC" | "GMT" | "Z"))
    {
        Some((head, _)) => (head, true),
        None => (zoneless, false),
    };
    let local = |naive: NaiveDateTime| -> Option<i64> {
        let zone = if utc_named { chrono_tz::UTC } else { timezone };
        match zone.from_local_datetime(&naive) {
            chrono::LocalResult::Single(date) => Some(date.timestamp()),
            // 回拨产生的重复小时：PHP timelib 随时区符号取舍——标准侧偏移为负取第一次
            // （夏令时侧），否则取第二次（标准侧）；与 America/New_York、Europe/Berlin、
            // Australia/Sydney 的实测一致
            chrono::LocalResult::Ambiguous(earliest, latest) => Some(
                if latest.offset().fix().local_minus_utc() < 0 {
                    earliest
                } else {
                    latest
                }
                .timestamp(),
            ),
            // 春令时跳跃产生的不存在本地时间：按跳跃后的偏移前移（与 PHP 一致）
            chrono::LocalResult::None => zone
                .from_local_datetime(&(naive + chrono::Duration::hours(1)))
                .earliest()
                .map(|date| date.timestamp()),
        }
    };
    for format in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y/%m/%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%a, %d %b %Y %H:%M:%S",
        // 小数秒与 12 小时制（PHP strtotime 接受）
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y/%m/%d %H:%M:%S%.f",
        "%Y-%m-%d %I:%M:%S %P",
        "%Y-%m-%d %I:%M:%S %p",
        "%Y/%m/%d %I:%M:%S %P",
        "%Y/%m/%d %I:%M:%S %p",
        // asctime 风格（周日与年份在后）
        "%a %b %d %H:%M:%S %Y",
    ] {
        if let Ok(date) = NaiveDateTime::parse_from_str(zoneless, format) {
            return local(date);
        }
    }
    for format in [
        "%Y-%m-%d",
        "%Y/%m/%d",
        "%d %b %Y",
        "%b %d, %Y",
        "%d.%m.%Y",
        "%Y%m%d",
    ] {
        if let Ok(date) = NaiveDate::parse_from_str(zoneless, format) {
            return local(date.and_hms_opt(0, 0, 0)?);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_dates_and_atom_title_types() {
        let shanghai = chrono_tz::Asia::Shanghai;
        for value in [
            "2026-11-26T12:00:00Z",
            "Thu, 26 Nov 2026 12:00:00 +0000",
            "2026-11-26 20:00:00 +0800",
            "2026-11-26 20:00:00+08:00",
            "2026-11-26 20:00:00",
            "2026-11-26T20:00:00",
            "2026/11/26 20:00:00",
            "2026-11-26 20:00",
            "Thu, 26 Nov 2026 20:00:00",
        ] {
            assert_eq!(timestamp(value, shanghai), Some(1795694400), "{value}");
        }
        assert_eq!(timestamp("2026-11-26", shanghai), Some(1795622400));
        assert_eq!(timestamp("2026/11/26", shanghai), Some(1795622400));
        assert_eq!(
            timestamp("2026-11-26 20:00:00", chrono_tz::UTC),
            Some(1795723200)
        );
        for value in ["", "not a date", "2026-02-30", "2026-11-26 25:00:00"] {
            assert_eq!(timestamp(value, shanghai), None);
        }
        for (markup, expected) in [
            (
                "<title>文字 &amp; &lt;literal&gt;</title>",
                "文字 & <literal>",
            ),
            (
                "<title type='html'>&lt;b&gt;文字&lt;/b&gt; &amp;amp; 标题</title>",
                "文字 & 标题",
            ),
            (
                "<title type='xhtml'><x:div xmlns:x='http://www.w3.org/1999/xhtml'>文字 <x:b>标题</x:b> &amp; &lt;literal&gt;</x:div></title>",
                "文字 标题 & <literal>",
            ),
        ] {
            let xml = format!("<entry xmlns='http://www.w3.org/2005/Atom'>{markup}</entry>");
            let doc = roxmltree::Document::parse(&xml).unwrap();
            assert_eq!(title(doc.root_element()), expected);
        }
    }
    #[test]
    fn declarations_bom_and_invalid_sequences() {
        for (encoding, label, text) in [
            (encoding_rs::GBK, "GB2312", "中文"),
            (encoding_rs::SHIFT_JIS, "Shift_JIS", "日本語"),
            (encoding_rs::WINDOWS_1252, "ISO-8859-1", "café"),
            (encoding_rs::EUC_JP, "EUC-JP", "日本語と行事"),
            (encoding_rs::BIG5, "Big5", "繁體中文"),
            (encoding_rs::EUC_KR, "EUC-KR", "한국어"),
            (encoding_rs::WINDOWS_1251, "windows-1251", "Кириллица"),
            (encoding_rs::UTF_8, "Utf-8", "大小写标签"),
            (encoding_rs::GBK, "gbk", "小写 gbk"),
        ] {
            let xml = format!("<?xml version='1.0' encoding='{label}'?><title>{text}</title>");
            let (bytes, _, errors) = encoding.encode(&xml);
            assert!(!errors);
            assert_eq!(decode(&bytes).unwrap(), xml);
        }
        let xml = "<?xml version='1.0' encoding='UTF-16'?><title>中文😀</title>";
        for big_endian in [false, true] {
            let bytes: Vec<_> = xml
                .encode_utf16()
                .flat_map(|unit| {
                    if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    }
                })
                .collect();
            assert_eq!(decode(&bytes).unwrap(), xml);
            let mut bom = if big_endian {
                vec![0xfe, 0xff]
            } else {
                vec![0xff, 0xfe]
            };
            bom.extend(bytes);
            assert_eq!(decode(&bom).unwrap(), xml);
        }
        assert_eq!(decode(b"\xef\xbb\xbf<feed/>").unwrap(), "<feed/>");
        assert!(decode(b"<?xml version='1.0' encoding='unknown'?><feed/>").is_err());
        assert!(decode(b"<feed>\xff</feed>").is_err());
        assert!(decode(b"\xff\xfe<\0x").is_err());
    }
}

#[cfg(test)]
mod strtotime_comparison {
    use super::*;

    /// 机器可读的日期格式与 PHP strtotime 逐一对照（自然语言如 now/+1 day 不在
    /// 支持范围，属于已记录的有意分歧）。
    #[test]
    fn machine_formats_match_php_strtotime() {
        let cases = [
            ("Asia/Shanghai", "2026-11-26T12:00:00Z"),
            ("Asia/Shanghai", "Thu, 26 Nov 2026 12:00:00 +0000"),
            ("Asia/Shanghai", "2026-11-26 20:00:00 +0800"),
            ("Asia/Shanghai", "2026-11-26 20:00:00+08:00"),
            ("Asia/Shanghai", "2026-11-26 20:00:00"),
            ("Asia/Shanghai", "2026-11-26T20:00:00"),
            ("Asia/Shanghai", "2026/11/26 20:00:00"),
            ("Asia/Shanghai", "2026-11-26 20:00"),
            ("Asia/Shanghai", "Thu, 26 Nov 2026 20:00:00"),
            ("Asia/Shanghai", "Thu, 26 Nov 2026 12:00:00 GMT"),
            ("Asia/Shanghai", "2026-11-26 12:00:00 UTC"),
            ("Asia/Shanghai", "2026-11-26T20:00:00.123+08:00"),
            ("Asia/Shanghai", "2026-11-26 20:00:00.123"),
            ("Asia/Shanghai", "Thu Nov 26 20:00:00 2026"),
            ("Asia/Shanghai", "2026-11-26 08:00:00 pm"),
            ("Asia/Shanghai", "2026-11-26 08:00:00 AM"),
            ("Asia/Shanghai", "26 Nov 2026"),
            ("Asia/Shanghai", "Nov 26, 2026"),
            ("Asia/Shanghai", "26.11.2026"),
            ("Asia/Shanghai", "20261126"),
            ("Asia/Shanghai", "2026-11-26"),
            ("Asia/Shanghai", "2026/11/26"),
            // 夏令时边界：回拨取第一次出现，跳跃前移一小时（与 PHP 一致）
            ("America/New_York", "2026-11-01 01:30:00"),
            ("America/New_York", "2026-03-08 02:30:00"),
            ("America/New_York", "2026-03-08 03:30:00"),
            ("Europe/Berlin", "2026-10-25 02:30:00"),
            ("Europe/Berlin", "2026-03-29 02:30:00"),
        ];
        for (zone, value) in cases {
            let output = std::process::Command::new("php")
                .args([
                    "-r",
                    "date_default_timezone_set($argv[1]); $t = strtotime($argv[2]); echo $t === false ? \"FALSE\" : $t;",
                    zone,
                    value,
                ])
                .output()
                .unwrap();
            assert!(output.status.success());
            let expected = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let actual = timestamp(value, zone.parse().unwrap()).map(|v| v.to_string());
            assert_eq!(actual.as_deref(), Some(expected.as_str()), "{zone} {value}");
        }
        // PHP 同样拒绝的输入（注意：PHP 会把 2026-02-30 顺延为 3 月 2 日，
        // Rust 按无效日期拒绝——已记录的有意分歧，真实订阅源不会出现）
        for (zone, value) in [
            ("Asia/Shanghai", ""),
            ("Asia/Shanghai", "not a date"),
            ("Asia/Shanghai", "2026-11-26 25:00:00"),
            ("Asia/Shanghai", "2026.11.26"),
        ] {
            let output = std::process::Command::new("php")
                .args([
                    "-r",
                    "date_default_timezone_set($argv[1]); $t = strtotime($argv[2]); echo $t === false ? \"FALSE\" : $t;",
                    zone,
                    value,
                ])
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "FALSE");
            assert_eq!(timestamp(value, zone.parse().unwrap()), None, "{value}");
        }
    }
}
