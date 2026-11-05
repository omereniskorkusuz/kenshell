//! CSV export of an analyzer table (RFC 4180; cells that a spreadsheet would run as a
//! formula are prefixed with `'`).

pub fn to_csv(headers: &[String], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    line(&mut out, headers);
    for r in rows {
        line(&mut out, r);
    }
    out
}

fn line(out: &mut String, cells: &[String]) {
    for (i, c) in cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        cell(out, c);
    }
    out.push_str("\r\n");
}

fn cell(out: &mut String, c: &str) {
    let guarded;
    let c = if c.starts_with(['=', '+', '@', '\t', '\r']) {
        guarded = format!("'{c}");
        guarded.as_str()
    } else {
        c
    };
    if c.contains([',', '"', '\n', '\r']) {
        out.push('"');
        out.push_str(&c.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_escapes_and_formula_guard() {
        let csv = to_csv(
            &["a".into(), "b".into()],
            &[
                vec!["x,y".into(), "say \"hi\"".into()],
                vec!["=cmd()".into(), "-".into()],
            ],
        );
        assert_eq!(csv, "a,b\r\n\"x,y\",\"say \"\"hi\"\"\"\r\n'=cmd(),-\r\n");
    }
}
