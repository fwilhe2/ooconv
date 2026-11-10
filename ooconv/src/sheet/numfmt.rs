// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Number formats between Excel's format codes (ECMA-376 §18.8.31) and ODF's data styles (ODF
//! 1.4 Part 3 §16.29), through [`NumFmt`]'s handful of shapes. A code that says more than its
//! shape is *simplified* and reported; a code with no shape is [`NumFmt::General`].

use super::model::NumFmt;

/// Excel's built-in format ids that need no `<numFmt>` (ECMA-376 §18.8.30, `SPEC`), for the
/// shapes that have one.
pub fn builtin_id(f: NumFmt) -> Option<u32> {
    Some(match f {
        NumFmt::General => 0,
        NumFmt::Fixed {
            decimals: 0,
            grouping: false,
        } => 1,
        NumFmt::Fixed {
            decimals: 2,
            grouping: false,
        } => 2,
        NumFmt::Fixed {
            decimals: 0,
            grouping: true,
        } => 3,
        NumFmt::Fixed {
            decimals: 2,
            grouping: true,
        } => 4,
        NumFmt::Percent { decimals: 0 } => 9,
        NumFmt::Percent { decimals: 2 } => 10,
        NumFmt::Scientific { decimals: 2 } => 11,
        NumFmt::Text => 49,
        _ => return None,
    })
}

/// A built-in id's shape, for reading. Ids 14–22 and 45–47 are dates and times whose code is
/// locale-dependent; the shape is not.
pub fn from_builtin(id: u32) -> Option<NumFmt> {
    Some(match id {
        0 => NumFmt::General,
        1 => NumFmt::Fixed {
            decimals: 0,
            grouping: false,
        },
        2 => NumFmt::Fixed {
            decimals: 2,
            grouping: false,
        },
        3 => NumFmt::Fixed {
            decimals: 0,
            grouping: true,
        },
        4 => NumFmt::Fixed {
            decimals: 2,
            grouping: true,
        },
        9 => NumFmt::Percent { decimals: 0 },
        10 => NumFmt::Percent { decimals: 2 },
        11 => NumFmt::Scientific { decimals: 2 },
        14..=17 => NumFmt::Date,
        18..=21 | 45..=47 => NumFmt::Time,
        22 => NumFmt::DateTime,
        49 => NumFmt::Text,
        _ => return None,
    })
}

/// The format code Excel is given for a shape without a built-in id.
pub fn excel_code(f: NumFmt) -> String {
    let decimals = |n: u8| match n {
        0 => String::new(),
        n => format!(".{}", "0".repeat(n.into())),
    };
    match f {
        NumFmt::General => "General".into(),
        NumFmt::Fixed {
            decimals: d,
            grouping,
        } => {
            format!("{}0{}", if grouping { "#,##" } else { "" }, decimals(d))
        }
        NumFmt::Percent { decimals: d } => format!("0{}%", decimals(d)),
        NumFmt::Scientific { decimals: d } => format!("0{}E+00", decimals(d)),
        NumFmt::Date => "yyyy\\-mm\\-dd".into(),
        NumFmt::Time => "hh:mm:ss".into(),
        NumFmt::DateTime => "yyyy\\-mm\\-dd hh:mm:ss".into(),
        NumFmt::Text => "@".into(),
    }
}

/// An Excel format code's shape, and whether the code said more than the shape carries.
pub fn classify_excel(code: &str) -> (NumFmt, bool) {
    // Only the first section speaks for positive numbers; later ones are negatives, zero, text.
    let mut first = String::new();
    let mut extra = false;
    let mut chars = code.chars().peekable();
    let mut sections = 0;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                extra = true;
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                }
            }
            '\\' => {
                // An escaped literal: a separator in a date is not "more".
                if let Some(lit) = chars.next()
                    && !matches!(lit, '-' | ' ' | '/' | ':' | '.')
                {
                    extra = true;
                }
                if sections == 0 {
                    first.push(' ');
                }
            }
            '[' => {
                let mut inner = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    inner.push(c);
                }
                let lower = inner.to_ascii_lowercase();
                // `[h]`, `[mm]`, `[ss]`: an elapsed duration — a time shape.
                if matches!(lower.as_str(), "h" | "hh" | "m" | "mm" | "s" | "ss") {
                    if sections == 0 {
                        first.push_str(&lower);
                    }
                } else if !lower.starts_with("$-") {
                    extra = true;
                }
            }
            ';' => {
                sections += 1;
                extra = true;
            }
            '_' | '*' => {
                chars.next();
            }
            c if sections == 0 => first.push(c),
            _ => {}
        }
    }
    let lower = first.to_ascii_lowercase();
    if lower.trim() == "general" || lower.trim().is_empty() {
        return (NumFmt::General, extra);
    }
    if lower.trim() == "@" {
        return (NumFmt::Text, extra);
    }
    let has_date = lower.contains('y') || lower.contains('d');
    let has_time = lower.contains('h') || lower.contains('s');
    if has_date || has_time {
        // An `m` alone is a month beside a date and a minute beside a time; either way the shape
        // follows from the other tokens.
        let shape = match (has_date, has_time) {
            (true, true) => NumFmt::DateTime,
            (true, false) => NumFmt::Date,
            _ => NumFmt::Time,
        };
        let canonical = matches!(
            lower
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .as_str(),
            "yyyy mm dd" | "hh:mm:ss" | "yyyy mm dd hh:mm:ss"
        );
        return (shape, extra || !canonical);
    }
    let decimals = lower
        .split_once('.')
        .map_or(0, |(_, after)| {
            after.chars().take_while(|c| *c == '0' || *c == '#').count()
        })
        .min(30) as u8;
    let digits = lower.chars().any(|c| matches!(c, '0' | '#' | '?'));
    if lower.contains('%') && digits {
        return (NumFmt::Percent { decimals }, extra);
    }
    if lower.contains("e+") || lower.contains("e-") {
        return (NumFmt::Scientific { decimals }, extra);
    }
    if digits {
        let grouping = lower.contains(',');
        let other = lower.chars().any(|c| !matches!(c, '0' | '#' | ',' | '.'));
        return (NumFmt::Fixed { decimals, grouping }, extra || other);
    }
    (NumFmt::General, true)
}

/// The ODF data style element for a shape, named `name`. `None` for General, which needs none.
pub fn odf_data_style(f: NumFmt, name: &str) -> Option<String> {
    let mut w = crate::xml::write::Writer::fragment();
    match f {
        NumFmt::General => return None,
        NumFmt::Fixed { decimals, grouping } => {
            w.start("number:number-style").attr("style:name", name);
            w.start("number:number")
                .attr("number:decimal-places", decimals.to_string())
                .attr("number:min-decimal-places", decimals.to_string())
                .attr("number:min-integer-digits", "1");
            if grouping {
                w.attr("number:grouping", "true");
            }
            w.end().end();
        }
        NumFmt::Percent { decimals } => {
            w.start("number:percentage-style").attr("style:name", name);
            w.start("number:number")
                .attr("number:decimal-places", decimals.to_string())
                .attr("number:min-decimal-places", decimals.to_string())
                .attr("number:min-integer-digits", "1")
                .end();
            w.leaf("number:text", "%");
            w.end();
        }
        NumFmt::Scientific { decimals } => {
            w.start("number:number-style").attr("style:name", name);
            w.start("number:scientific-number")
                .attr("number:decimal-places", decimals.to_string())
                .attr("number:min-decimal-places", decimals.to_string())
                .attr("number:min-integer-digits", "1")
                .attr("number:min-exponent-digits", "2")
                .end();
            w.end();
        }
        NumFmt::Date | NumFmt::DateTime => {
            w.start("number:date-style").attr("style:name", name);
            w.start("number:year").attr("number:style", "long").end();
            w.leaf("number:text", "-");
            w.start("number:month").attr("number:style", "long").end();
            w.leaf("number:text", "-");
            w.start("number:day").attr("number:style", "long").end();
            if f == NumFmt::DateTime {
                w.leaf("number:text", " ");
                time_parts(&mut w);
            }
            w.end();
        }
        NumFmt::Time => {
            w.start("number:time-style")
                .attr("style:name", name)
                .attr("number:truncate-on-overflow", "false");
            time_parts(&mut w);
            w.end();
        }
        NumFmt::Text => {
            w.start("number:text-style").attr("style:name", name);
            w.start("number:text-content").end();
            w.end();
        }
    }
    Some(w.finish())
}

fn time_parts(w: &mut crate::xml::write::Writer) {
    w.start("number:hours").attr("number:style", "long").end();
    w.leaf("number:text", ":");
    w.start("number:minutes").attr("number:style", "long").end();
    w.leaf("number:text", ":");
    w.start("number:seconds").attr("number:style", "long").end();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_classifies_as_itself_from_the_code_it_writes() {
        for f in [
            NumFmt::General,
            NumFmt::Fixed {
                decimals: 0,
                grouping: false,
            },
            NumFmt::Fixed {
                decimals: 3,
                grouping: true,
            },
            NumFmt::Percent { decimals: 1 },
            NumFmt::Scientific { decimals: 2 },
            NumFmt::Date,
            NumFmt::Time,
            NumFmt::DateTime,
            NumFmt::Text,
        ] {
            assert_eq!(
                classify_excel(&excel_code(f)),
                (f, false),
                "{}",
                excel_code(f)
            );
            if let Some(id) = builtin_id(f) {
                assert_eq!(from_builtin(id), Some(f));
            }
        }
    }

    #[test]
    fn what_real_codes_mean() {
        assert_eq!(classify_excel("m/d/yyyy").0, NumFmt::Date);
        assert_eq!(classify_excel("[h]:mm:ss").0, NumFmt::Time);
        assert_eq!(classify_excel("mm:ss").0, NumFmt::Time);
        assert_eq!(classify_excel("d-mmm-yy h:mm").0, NumFmt::DateTime);
        assert_eq!(
            classify_excel("#,##0.00;[Red]-#,##0.00"),
            (
                NumFmt::Fixed {
                    decimals: 2,
                    grouping: true
                },
                true
            )
        );
        assert!(
            classify_excel("\"$\"#,##0").1,
            "a currency is more than its shape"
        );
        assert_eq!(
            classify_excel("[$-409]yyyy\\-mm\\-dd"),
            (NumFmt::Date, false)
        );
    }
}
