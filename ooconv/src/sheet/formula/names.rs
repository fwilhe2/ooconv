// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Function names between the two dialects. The model holds the **OpenFormula** name; each table
//! here is measured (`doc/xlsx-format.md` §3):
//!
//! - Excel → ODF: grind's import tables (`xlsx/src/formula.rs`, measured against LibreOffice's
//!   import, grind `doc/xlsx-format.md` §5.1).
//! - ODF → Excel: LibreOffice's xlsx export of every name below, vendored as
//!   `tests/data/measured/function-names-lo-export.txt` and checked against this file by a test.
//!
//! **Semantics are never translated.** Excel's two-argument `CEILING` is not OpenFormula's, so it
//! travels as `COM.MICROSOFT.CEILING`, which an ODF consumer evaluates as Excel does.

/// Excel's functions OpenFormula has no name for, carried under the `COM.MICROSOFT.` prefix.
pub const MICROSOFT: &[&str] = &[
    "CEILING",
    "FLOOR",
    "AGGREGATE",
    "BETA.DIST",
    "BETA.INV",
    "BINOM.DIST",
    "BINOM.INV",
    "CEILING.MATH",
    "CEILING.PRECISE",
    "CHISQ.DIST",
    "CHISQ.DIST.RT",
    "CHISQ.INV",
    "CHISQ.INV.RT",
    "CHISQ.TEST",
    "CHOOSECOLS",
    "CHOOSEROWS",
    "CONCAT",
    "CONFIDENCE.NORM",
    "CONFIDENCE.T",
    "COVARIANCE.P",
    "COVARIANCE.S",
    "DROP",
    "ENCODEURL",
    "ERF.PRECISE",
    "ERFC.PRECISE",
    "EXPAND",
    "EXPON.DIST",
    "F.DIST.RT",
    "F.INV.RT",
    "F.TEST",
    "FILTER",
    "FILTERXML",
    "FLOOR.MATH",
    "FLOOR.PRECISE",
    "FORECAST.ETS",
    "FORECAST.ETS.CONFINT",
    "FORECAST.ETS.SEASONALITY",
    "FORECAST.ETS.STAT",
    "FORECAST.LINEAR",
    "GAMMA.DIST",
    "GAMMA.INV",
    "GAMMALN.PRECISE",
    "HSTACK",
    "HYPGEOM.DIST",
    "IFS",
    "LET",
    "LOGNORM.DIST",
    "LOGNORM.INV",
    "MAXIFS",
    "MINIFS",
    "MODE.MULT",
    "MODE.SNGL",
    "NEGBINOM.DIST",
    "NORM.DIST",
    "NORM.INV",
    "NORM.S.DIST",
    "NORM.S.INV",
    "PERCENTILE.EXC",
    "PERCENTILE.INC",
    "PERCENTRANK.EXC",
    "PERCENTRANK.INC",
    "POISSON.DIST",
    "QUARTILE.EXC",
    "QUARTILE.INC",
    "RANDARRAY",
    "RANK.AVG",
    "RANK.EQ",
    "SEQUENCE",
    "SORT",
    "SORTBY",
    "STDEV.P",
    "STDEV.S",
    "SWITCH",
    "T.DIST",
    "T.DIST.2T",
    "T.DIST.RT",
    "T.INV",
    "T.INV.2T",
    "T.TEST",
    "TAKE",
    "TEXTAFTER",
    "TEXTBEFORE",
    "TEXTJOIN",
    "TEXTSPLIT",
    "TOCOL",
    "TOROW",
    "UNIQUE",
    "VAR.P",
    "VAR.S",
    "VSTACK",
    "WEBSERVICE",
    "WEIBULL.DIST",
    "WRAPCOLS",
    "WRAPROWS",
    "XLOOKUP",
    "XMATCH",
    "Z.TEST",
    "ARRAYTOTEXT",
    "BAHTTEXT",
    "BYCOL",
    "BYROW",
    "ISO.CEILING",
    "ISOMITTED",
    "LAMBDA",
    "MAKEARRAY",
    "MAP",
    "NETWORKDAYS.INTL",
    "REDUCE",
    "SCAN",
    "WORKDAY.INTL",
];

/// Excel's pre-2010 distributions, which ODF keeps under `LEGACY.`.
pub const LEGACY: &[&str] = &[
    "CHIDIST",
    "CHIINV",
    "CHITEST",
    "FDIST",
    "FINV",
    "NORMSDIST",
    "NORMSINV",
    "TDIST",
];

/// Excel 2010's names for functions OpenFormula already had: `(excel, odf)`.
pub const ODF_NAMED: &[(&str, &str)] = &[
    ("F.DIST", "FDIST"),
    ("F.INV", "FINV"),
    ("FORMULATEXT", "FORMULA"),
    ("SKEW.P", "SKEWP"),
];

/// Functions Excel's file format writes with `_xlfn.` because they are newer than Excel 2007,
/// by their **Excel** name — measured as LibreOffice's export writes them.
pub const FUTURE: &[&str] = &[
    "AGGREGATE",
    "BETA.DIST",
    "BETA.INV",
    "BINOM.DIST",
    "BINOM.INV",
    "CEILING.MATH",
    "CEILING.PRECISE",
    "CHISQ.DIST",
    "CHISQ.DIST.RT",
    "CHISQ.INV",
    "CHISQ.INV.RT",
    "CHISQ.TEST",
    "CHOOSECOLS",
    "CHOOSEROWS",
    "CONCAT",
    "CONFIDENCE.NORM",
    "CONFIDENCE.T",
    "COVARIANCE.P",
    "COVARIANCE.S",
    "DROP",
    "ENCODEURL",
    "ERF.PRECISE",
    "ERFC.PRECISE",
    "EXPAND",
    "EXPON.DIST",
    "F.DIST.RT",
    "F.INV.RT",
    "F.TEST",
    "FILTER",
    "FILTERXML",
    "FLOOR.MATH",
    "FLOOR.PRECISE",
    "FORECAST.ETS",
    "FORECAST.ETS.CONFINT",
    "FORECAST.ETS.SEASONALITY",
    "FORECAST.ETS.STAT",
    "FORECAST.LINEAR",
    "GAMMA.DIST",
    "GAMMA.INV",
    "GAMMALN.PRECISE",
    "HSTACK",
    "HYPGEOM.DIST",
    "IFS",
    "LET",
    "LOGNORM.DIST",
    "LOGNORM.INV",
    "MAXIFS",
    "MINIFS",
    "MODE.MULT",
    "MODE.SNGL",
    "NEGBINOM.DIST",
    "NORM.DIST",
    "NORM.INV",
    "NORM.S.DIST",
    "NORM.S.INV",
    "PERCENTILE.EXC",
    "PERCENTILE.INC",
    "PERCENTRANK.EXC",
    "PERCENTRANK.INC",
    "POISSON.DIST",
    "QUARTILE.EXC",
    "QUARTILE.INC",
    "RANDARRAY",
    "RANK.AVG",
    "RANK.EQ",
    "SEQUENCE",
    "SORT",
    "SORTBY",
    "STDEV.P",
    "STDEV.S",
    "SWITCH",
    "T.DIST",
    "T.DIST.2T",
    "T.DIST.RT",
    "T.INV",
    "T.INV.2T",
    "T.TEST",
    "TAKE",
    "TEXTAFTER",
    "TEXTBEFORE",
    "TEXTJOIN",
    "TEXTSPLIT",
    "TOCOL",
    "TOROW",
    "UNIQUE",
    "VAR.P",
    "VAR.S",
    "VSTACK",
    "WEBSERVICE",
    "WEIBULL.DIST",
    "WRAPCOLS",
    "WRAPROWS",
    "XLOOKUP",
    "XMATCH",
    "Z.TEST",
    "BAHTTEXT",
    "F.DIST",
    "F.INV",
    "FORMULATEXT",
    "SKEW.P",
    "IFNA",
    "XOR",
    "DAYS",
    "ISOWEEKNUM",
    "SHEET",
    "SHEETS",
    "ISFORMULA",
    "NUMBERVALUE",
    "UNICHAR",
    "UNICODE",
    "ARABIC",
    "BASE",
    "DECIMAL",
    "BITAND",
    "BITOR",
    "BITXOR",
    "BITLSHIFT",
    "BITRSHIFT",
    "COT",
    "COTH",
    "CSC",
    "CSCH",
    "SEC",
    "SECH",
    "ACOT",
    "ACOTH",
    "COMBINA",
    "PERMUTATIONA",
    "PHI",
    "GAUSS",
    "PDURATION",
    "RRI",
    "GAMMA",
    "MUNIT",
];

/// Worksheet-only functions, which carry a second marker after `_xlfn.` — measured.
pub const WORKSHEET: &[&str] = &["FILTER", "SORT"];

/// OpenFormula names whose Excel file spelling is not measured — LibreOffice cannot export them
/// either. The xlsx writer refuses a formula naming one rather than guess (`UNVERIFIED` rule).
pub const UNSPELLED: &[&str] = &[
    "COM.MICROSOFT.ARRAYTOTEXT",
    "COM.MICROSOFT.BYCOL",
    "COM.MICROSOFT.BYROW",
    "COM.MICROSOFT.ISOMITTED",
    "COM.MICROSOFT.LAMBDA",
    "COM.MICROSOFT.MAKEARRAY",
    "COM.MICROSOFT.MAP",
    "COM.MICROSOFT.REDUCE",
    "COM.MICROSOFT.SCAN",
];

/// The name a formula carries in ODF: Excel's name without its markers, uppercased, renamed where
/// OpenFormula spells the function differently.
pub fn excel_to_odf(word: &str) -> String {
    let mut name = word;
    for marker in ["_xlfn.", "_xlws."] {
        name = name.strip_prefix(marker).unwrap_or(name);
    }
    let name = name.to_ascii_uppercase();
    if MICROSOFT.contains(&name.as_str()) {
        format!("COM.MICROSOFT.{name}")
    } else if LEGACY.contains(&name.as_str()) {
        format!("LEGACY.{name}")
    } else if let Some((_, odf)) = ODF_NAMED.iter().find(|(excel, _)| *excel == name) {
        (*odf).to_owned()
    } else {
        name
    }
}

/// The name Excel's file format needs for an OpenFormula function, markers included.
pub fn odf_to_excel(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    let excel = if let Some(rest) = upper.strip_prefix("COM.MICROSOFT.") {
        rest.to_owned()
    } else if let Some(rest) = upper.strip_prefix("LEGACY.") {
        rest.to_owned()
    } else if let Some((excel, _)) = ODF_NAMED.iter().find(|(_, odf)| *odf == upper) {
        (*excel).to_owned()
    } else {
        upper
    };
    if WORKSHEET.contains(&excel.as_str()) {
        format!("_xlfn._xlws.{excel}")
    } else if FUTURE.contains(&excel.as_str()) {
        format!("_xlfn.{excel}")
    } else {
        excel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measurement, replayed: every name LibreOffice exported is exported the same way here,
    /// apart from the named divergences.
    #[test]
    fn the_export_matches_the_measured_oracle() {
        // LibreOffice writes these lower-case and unprefixed: names it cannot export. Excel's
        // spelling is `UNVERIFIED` here, so the writer refuses them rather than guess.
        const DIVERGES: &[&str] = UNSPELLED;
        // Not an OpenFormula name — ODF's is `B` — so the oracle read it as `B`.
        const NOT_ODF: &[&str] = &["BINOM.DIST.RANGE"];
        let table = include_str!("../../../tests/data/measured/function-names-lo-export.txt");
        let mut checked = 0;
        for line in table.lines().filter(|l| !l.starts_with('#')) {
            let (odf, excel) = line.split_once('|').expect("odf|excel");
            if NOT_ODF.contains(&odf) {
                continue;
            }
            if DIVERGES.contains(&odf) {
                assert!(excel.starts_with("com.microsoft."), "{odf}: {excel}");
                continue;
            }
            assert_eq!(odf_to_excel(odf), excel, "{odf}");
            checked += 1;
        }
        assert!(checked >= 170, "the measurement shrank: {checked}");
    }

    #[test]
    fn excel_and_odf_names_round_trip() {
        for excel in [
            "_xlfn.XLOOKUP",
            "CEILING",
            "NORMSDIST",
            "_xlfn.F.DIST",
            "SUM",
            "_xlfn._xlws.SORT",
        ] {
            assert_eq!(odf_to_excel(&excel_to_odf(excel)), excel);
        }
    }

    /// Unknown names travel as they are, so a user-defined function survives.
    #[test]
    fn an_unknown_name_is_carried() {
        assert_eq!(excel_to_odf("MYUDF"), "MYUDF");
        assert_eq!(odf_to_excel("MYUDF"), "MYUDF");
    }
}
