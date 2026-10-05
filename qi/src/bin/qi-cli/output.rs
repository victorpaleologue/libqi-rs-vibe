//! Rendering of values, services and meta objects, either for humans or as JSON.

use crate::{
    json,
    meta::{self, MemberFilter},
};
use colored::Colorize;
use qi::{
    object::MetaObject,
    service::Info,
    value::{Type, Value},
};
use serde_json::Value as Json;
use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Renders a value for printing on its own.
///
/// The value is rendered as a JSON document, unless human-readable output is requested and the
/// value has a more natural rendering: strings are printed as they are, raw data as a hexadecimal
/// dump, and nothing is printed for a unit value (`None`).
pub(crate) fn render(value: &Value<'_>, ty: Option<&Type>, as_json: bool) -> Option<String> {
    if as_json {
        return Some(pretty_json(&json::from_value(value, ty)));
    }
    match value {
        Value::Unit => None,
        Value::Dynamic(inner) => render(inner, None, false),
        Value::String(text) => Some(json::string_lossy(text)),
        Value::Raw(bytes) => Some(hexdump(bytes)),
        other => Some(pretty_json(&json::from_value(other, ty))),
    }
}

/// Renders a value on a single line, for streams of events.
pub(crate) fn render_line(value: &Value<'_>, ty: Option<&Type>, as_json: bool) -> String {
    if as_json {
        return json::from_value(value, ty).to_string();
    }
    match value {
        Value::Unit => "()".to_owned(),
        Value::Dynamic(inner) => render_line(inner, None, false),
        Value::String(text) => json::string_lossy(text),
        Value::Raw(bytes) => format!("raw({} bytes) {}", bytes.len(), json::base64(bytes)),
        other => json::from_value(other, ty).to_string(),
    }
}

pub(crate) fn pretty_json(json: &Json) -> String {
    serde_json::to_string_pretty(json).unwrap_or_else(|_| json.to_string())
}

/// A hexadecimal dump of raw data, 16 bytes per line with their printable ASCII rendering.
pub(crate) fn hexdump(bytes: &[u8]) -> String {
    const BYTES_PER_LINE: usize = 16;
    // Two hexadecimal digits and a space per byte, plus a space in the middle of the line.
    const HEX_WIDTH: usize = BYTES_PER_LINE * 3 + 1;
    let mut lines = vec![format!("raw ({} bytes)", bytes.len())];
    for (index, chunk) in bytes.chunks(BYTES_PER_LINE).enumerate() {
        let hex: String = chunk
            .iter()
            .enumerate()
            .map(|(i, byte)| format!("{}{byte:02x}", if i == 8 { "  " } else { " " }))
            .collect();
        let padding = " ".repeat(HEX_WIDTH - (chunk.len() * 3 + usize::from(chunk.len() > 8)));
        let ascii: String = chunk
            .iter()
            .map(|&byte| {
                if byte.is_ascii_graphic() || byte == b' ' {
                    char::from(byte)
                } else {
                    '.'
                }
            })
            .collect();
        lines.push(format!(
            "{:08x} {hex}{padding}  |{ascii}|",
            index * BYTES_PER_LINE
        ));
    }
    lines.join("\n")
}

/// Formats a time as an ISO 8601 UTC timestamp with milliseconds, e.g.
/// `2026-09-29T16:42:03.123Z`.
pub(crate) fn utc_timestamp(time: SystemTime) -> String {
    const SECONDS_PER_DAY: i64 = 86_400;
    let since_epoch = time.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let seconds = i64::try_from(since_epoch.as_secs()).unwrap_or(i64::MAX);
    let (year, month, day) = civil_from_days(seconds.div_euclid(SECONDS_PER_DAY));
    let day_seconds = seconds.rem_euclid(SECONDS_PER_DAY);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        day_seconds.div_euclid(3600),
        day_seconds.rem_euclid(3600).div_euclid(60),
        day_seconds.rem_euclid(60),
        since_epoch.subsec_millis()
    )
}

/// Converts a number of days since the Unix epoch to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era.div_euclid(1460) + day_of_era.div_euclid(36_524)
        - day_of_era.div_euclid(146_096))
    .div_euclid(365);
    let year = year_of_era + era * 400;
    let day_of_year =
        day_of_era - (365 * year_of_era + year_of_era.div_euclid(4) - year_of_era.div_euclid(100));
    let shifted_month = (5 * day_of_year + 2).div_euclid(153);
    let day = day_of_year - (153 * shifted_month + 2).div_euclid(5) + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// The meta object of a service, or the reason it could not be obtained. `None` when it was not
/// requested.
pub(crate) type MetaResult<'a> = Option<Result<&'a MetaObject, String>>;

/// The human-readable report of a service, in the style of `qicli info`.
pub(crate) struct ServiceReport<'a> {
    pub(crate) info: &'a Info,
    pub(crate) meta: MetaResult<'a>,
    pub(crate) filter: MemberFilter,
}

impl fmt::Display for ServiceReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let info = self.info;
        writeln!(
            f,
            "{} [{}]",
            format!("{:03}", u32::from(info.id())).magenta(),
            info.name().red()
        )?;
        let Some(meta) = &self.meta else {
            return Ok(());
        };
        writeln!(f, "  {}", "* Info:".green())?;
        writeln!(f, "    {:<10}{}", "machine".bold(), info.machine_id())?;
        writeln!(f, "    {:<10}{}", "process".bold(), info.process_id())?;
        for (index, endpoint) in info.endpoints().iter().enumerate() {
            let label = if index == 0 { "endpoints" } else { "" };
            writeln!(f, "    {:<10}{endpoint}", label.bold())?;
        }
        if self.filter.details {
            writeln!(f, "    {:<10}{}", "node".bold(), info.node_uid())?;
            let object_uid = info
                .object_uid()
                .map_or_else(|| "unknown".to_owned(), |uid| uid.to_string());
            writeln!(f, "    {:<10}{object_uid}", "object".bold())?;
        }
        match meta {
            Ok(meta) => MetaReport {
                meta,
                filter: self.filter,
            }
            .fmt(f),
            Err(error) => writeln!(f, "  {} {error}", "* Error:".red()),
        }
    }
}

/// The human-readable report of the members of a meta object.
pub(crate) struct MetaReport<'a> {
    pub(crate) meta: &'a MetaObject,
    pub(crate) filter: MemberFilter,
}

impl fmt::Display for MetaReport<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const INDENT: &str = "          ";
        let meta = self.meta;
        let details = self.filter.details;
        if !meta.description.is_empty() {
            writeln!(f, "  {} {}", "* Description:".green(), meta.description)?;
        }
        let methods = self.filter.methods(meta);
        if !methods.is_empty() {
            writeln!(f, "  {}", "* Methods:".green())?;
        }
        for method in methods {
            writeln!(
                f,
                "    {} {}",
                format!("{:03}", u32::from(method.uid)).magenta(),
                meta::method_signature(method)
            )?;
            if details {
                writeln!(
                    f,
                    "{INDENT}{:<12}{}",
                    "signature".dimmed(),
                    meta::method_raw_signature(method)
                )?;
                if !method.description.is_empty() {
                    writeln!(
                        f,
                        "{INDENT}{:<12}{}",
                        "description".dimmed(),
                        method.description
                    )?;
                }
                for parameter in &method.parameters {
                    writeln!(
                        f,
                        "{INDENT}{:<12}{}: {}",
                        "parameter".dimmed(),
                        parameter.name,
                        parameter.description
                    )?;
                }
                if !method.return_description.is_empty() {
                    writeln!(
                        f,
                        "{INDENT}{:<12}{}",
                        "returns".dimmed(),
                        method.return_description
                    )?;
                }
            }
        }
        let signals = self.filter.signals(meta);
        if !signals.is_empty() {
            writeln!(f, "  {}", "* Signals:".green())?;
        }
        for signal in signals {
            writeln!(
                f,
                "    {} {}",
                format!("{:03}", u32::from(signal.uid)).magenta(),
                meta::signal_signature(signal)
            )?;
            if details {
                writeln!(
                    f,
                    "{INDENT}{:<12}{}",
                    "signature".dimmed(),
                    signal.signature
                )?;
            }
        }
        let properties = self.filter.properties(meta);
        if !properties.is_empty() {
            writeln!(f, "  {}", "* Properties:".green())?;
        }
        for property in properties {
            writeln!(
                f,
                "    {} {}",
                format!("{:03}", u32::from(property.uid)).magenta(),
                meta::property_signature(property)
            )?;
            if details {
                writeln!(
                    f,
                    "{INDENT}{:<12}{}",
                    "signature".dimmed(),
                    property.signature
                )?;
            }
        }
        Ok(())
    }
}

/// The JSON report of a service.
pub(crate) fn service_json(info: &Info, meta: &MetaResult<'_>, filter: MemberFilter) -> Json {
    let mut json = serde_json::json!({
        "serviceId": u32::from(info.id()),
        "name": info.name(),
        "machineId": info.machine_id().to_string(),
        "processId": info.process_id(),
        "endpoints": info.endpoints().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "nodeUid": info.node_uid().to_string(),
        "objectUid": info.object_uid().map(|uid| uid.to_string()),
    });
    match meta {
        Some(Ok(meta)) => {
            json["description"] = Json::String(meta.description.clone());
            json["methods"] = filter
                .methods(meta)
                .into_iter()
                .map(|method| {
                    serde_json::json!({
                        "uid": u32::from(method.uid),
                        "name": method.name,
                        "signature": meta::method_signature(method),
                        "parametersSignature": method.parameters_signature.to_string(),
                        "returnSignature": method.return_signature.to_string(),
                        "description": method.description,
                        "parameters": method.parameters.iter().map(|parameter| serde_json::json!({
                            "name": parameter.name,
                            "description": parameter.description,
                        })).collect::<Vec<_>>(),
                        "returnDescription": method.return_description,
                    })
                })
                .collect();
            json["signals"] = filter
                .signals(meta)
                .into_iter()
                .map(|signal| {
                    serde_json::json!({
                        "uid": u32::from(signal.uid),
                        "name": signal.name,
                        "signature": signal.signature.to_string(),
                    })
                })
                .collect();
            json["properties"] = filter
                .properties(meta)
                .into_iter()
                .map(|property| {
                    serde_json::json!({
                        "uid": u32::from(property.uid),
                        "name": property.name,
                        "signature": property.signature.to_string(),
                    })
                })
                .collect();
        }
        Some(Err(error)) => json["error"] = Json::String(error.clone()),
        None => {}
    }
    json
}

#[cfg(test)]
mod tests {
    use super::*;
    use qi::value::IntoValue;

    #[test]
    fn values_render_for_humans_or_as_json() {
        let text = Value::String("hello".to_owned().into());
        assert_eq!(render(&text, None, false).unwrap(), "hello");
        assert_eq!(render(&text, None, true).unwrap(), "\"hello\"");
        assert_eq!(render(&Value::Unit, None, false), None);
        assert_eq!(render(&Value::Unit, None, true).unwrap(), "null");
        assert_eq!(render(&Value::Int32(3), None, false).unwrap(), "3");
        assert_eq!(
            render(&Value::Dynamic(Box::new(text.clone())), None, false).unwrap(),
            "hello"
        );
        assert_eq!(
            render(&Value::List(vec![Value::Int32(1)]), None, false).unwrap(),
            "[\n  1\n]"
        );
        let raw = Value::Raw(vec![0, 1].into());
        assert!(render(&raw, None, false)
            .unwrap()
            .starts_with("raw (2 bytes)\n"));
        assert_eq!(render(&raw, None, true).unwrap(), "\"AAE=\"");

        assert_eq!(render_line(&text, None, false), "hello");
        assert_eq!(render_line(&text, None, true), "\"hello\"");
        assert_eq!(render_line(&Value::Unit, None, false), "()");
        assert_eq!(render_line(&Value::Unit, None, true), "null");
        assert_eq!(render_line(&raw, None, false), "raw(2 bytes) AAE=");
        assert_eq!(
            render_line(
                &Value::List(vec![Value::Int32(1), 0.5f32.into_value()]),
                None,
                false
            ),
            "[1,0.5]"
        );
    }

    #[test]
    fn raw_data_dumps_in_hexadecimal() {
        let dump = hexdump(b"Hello, world! This is raw data.\x00\xff");
        let lines: Vec<_> = dump.lines().collect();
        assert_eq!(lines[0], "raw (33 bytes)");
        assert_eq!(
            lines[1],
            "00000000  48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 21 20 54 68  |Hello, world! Th|"
        );
        assert_eq!(
            lines[2],
            "00000010  69 73 20 69 73 20 72 61  77 20 64 61 74 61 2e 00  |is is raw data..|"
        );
        assert_eq!(
            lines[3],
            "00000020  ff                                                |.|"
        );
        assert_eq!(hexdump(&[]), "raw (0 bytes)");
    }

    #[test]
    fn timestamps_are_iso_8601_utc() {
        let at = |seconds: u64, millis: u32| {
            utc_timestamp(
                UNIX_EPOCH + Duration::from_secs(seconds) + Duration::from_millis(millis.into()),
            )
        };
        assert_eq!(at(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(at(951_782_400, 0), "2000-02-29T00:00:00.000Z");
        assert_eq!(at(1_700_000_000, 234), "2023-11-14T22:13:20.234Z");
        assert_eq!(at(4_102_444_799, 999), "2099-12-31T23:59:59.999Z");
    }
}
