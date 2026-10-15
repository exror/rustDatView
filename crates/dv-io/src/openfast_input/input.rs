use dv_core::{Column, DataFrame, FileObject, IoError, NamedFrame};
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq)]
pub enum InputValue {
    Bool(bool),
    Integer(i64),
    Number(f64),
    Text(String),
    List(Vec<InputValue>),
    Table(InputTable),
}
impl InputValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
            Self::Integer(v) => Some(*v as f64),
            Self::Number(v) => Some(*v),
            Self::Text(s) => float(s),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Self::Text(s) = self {
            Some(s.trim_matches(['"', '\'']))
        } else {
            None
        }
    }
}
impl fmt::Display for InputValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(v) => write!(f, "{}", if *v { "True" } else { "False" }),
            Self::Integer(v) => write!(f, "{v}"),
            Self::Number(v) => write!(f, "{v}"),
            Self::Text(v) => write!(f, "{v}"),
            Self::List(v) => {
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{x}")?;
                }
                Ok(())
            }
            Self::Table(_) => Err(fmt::Error),
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct InputTable {
    pub columns: Vec<String>,
    pub units: Vec<String>,
    pub rows: Vec<Vec<InputValue>>,
}
impl InputTable {
    pub fn numeric_rows(&self) -> Result<Vec<Vec<f64>>, IoError> {
        self.rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|v| v.as_f64().ok_or_else(|| broken("non-numeric table cell")))
                    .collect()
            })
            .collect()
    }
    pub fn column(&self, name: &str) -> Option<Vec<f64>> {
        let i = self
            .columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))?;
        self.rows.iter().map(|r| r.get(i)?.as_f64()).collect()
    }
    pub fn dataframe(&self) -> Result<DataFrame, IoError> {
        let mut cols = Vec::with_capacity(self.columns.len());
        for (i, c) in self.columns.iter().enumerate() {
            let name = if let Some(u) = self.units.get(i) {
                format!("{}_{}", c, u.replace('(', "[").replace(')', "]"))
            } else {
                c.clone()
            };
            let values: Vec<&InputValue> = self
                .rows
                .iter()
                .map(|r| r.get(i).ok_or_else(|| broken("ragged table")))
                .collect::<Result<_, _>>()?;
            if values.iter().all(|v| v.as_f64().is_some()) {
                cols.push(Column::float(
                    name,
                    values.iter().map(|v| v.as_f64().unwrap()).collect(),
                ));
            } else {
                cols.push(Column::string(
                    name,
                    values.iter().map(|v| v.to_string()).collect(),
                ));
            }
        }
        DataFrame::from_columns(cols).map_err(|e| broken(e.to_string()))
    }
}
#[derive(Debug, Clone)]
pub struct InputEntry {
    pub label: String,
    pub value: InputValue,
    pub description: String,
    pub is_comment: bool,
    pub dimension: Option<String>,
    pub table_type: u32,
}
#[derive(Debug, Clone)]
pub struct FastInputFile {
    pub filename: PathBuf,
    pub module: Option<String>,
    pub entries: Vec<InputEntry>,
    pub has_nodal: bool,
}
pub(crate) fn broken(s: impl Into<String>) -> IoError {
    IoError::BrokenFormat(s.into())
}
pub(crate) fn float(s: &str) -> Option<f64> {
    s.replace(['d', 'D'], "E").parse().ok()
}
pub(crate) fn atom(s: &str) -> InputValue {
    match s.to_ascii_lowercase().as_str() {
        "true" | "t" | "print" => InputValue::Bool(true),
        "false" | "f" | "noprint" => InputValue::Bool(false),
        _ => {
            if let Ok(v) = s.parse::<i64>() {
                InputValue::Integer(v)
            } else if let Some(v) = float(s) {
                InputValue::Number(v)
            } else {
                InputValue::Text(s.to_string())
            }
        }
    }
}
pub(crate) fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    for c in s.chars() {
        if let Some(q) = quote {
            token.push(c);
            if c == q {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
            token.push(c);
        } else if c.is_whitespace() {
            if !token.is_empty() {
                out.push(std::mem::take(&mut token));
            }
        } else {
            token.push(c);
        }
    }
    if !token.is_empty() {
        out.push(token);
    }
    out
}
pub(crate) fn read_text(path: &Path) -> Result<String, IoError> {
    let bytes = fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            IoError::NotFound(path.into())
        } else {
            e.into()
        }
    })?;
    if bytes.is_empty() {
        return Err(IoError::Empty(path.into()));
    }
    if bytes.contains(&0) {
        return Err(IoError::WrongFormat("binary input".into()));
    }
    Ok(String::from_utf8_lossy(&bytes)
        .trim_start_matches('\u{feff}')
        .to_string())
}
fn entry(label: impl Into<String>, value: InputValue) -> InputEntry {
    InputEntry {
        label: label.into(),
        value,
        description: String::new(),
        is_comment: false,
        dimension: None,
        table_type: 0,
    }
}
fn comment(s: &str) -> InputEntry {
    let mut e = entry("", InputValue::Text(s.into()));
    e.is_comment = true;
    e
}
fn parse_line(s: &str, space_list: bool) -> InputEntry {
    let t = s.trim();
    if t.is_empty()
        || ["!", "#", "--", "==", "END of", "END ("]
            .iter()
            .any(|p| t.starts_with(*p))
        || t.eq_ignore_ascii_case("END")
    {
        return comment(s);
    }
    let ts = tokens(t);
    if ts.len() < 2 {
        return comment(s);
    }
    let mut values = Vec::new();
    let mut next = 0;
    if t.find(',').is_some_and(|i| i < 30) {
        while next < ts.len() {
            let pieces: Vec<_> = ts[next].split(',').filter(|s| !s.is_empty()).collect();
            if pieces.is_empty() || pieces.iter().any(|p| atom(p).as_f64().is_none()) {
                break;
            }
            values.extend(pieces.into_iter().map(atom));
            next += 1;
        }
    }
    let value = if values.len() >= 2 {
        InputValue::List(values)
    } else {
        next = 1;
        atom(&ts[0])
    };
    if space_list && next == 1 && ts.get(1).is_some_and(|s| s.parse::<i64>().is_ok()) {
        let mut e = entry(
            ts[1].clone(),
            InputValue::Text(format!("{} {}", ts[0], ts[1])),
        );
        e.description = ts[2..].join(" ");
        return e;
    }
    while next < ts.len() {
        if ts[next].starts_with('!') && ts[next].ends_with(':') {
            next += 2;
        } else if float(&ts[next]).is_some() {
            next += 1;
        } else {
            break;
        }
    }
    if next >= ts.len() {
        return comment(s);
    }
    let mut e = entry(&ts[next], value);
    e.description = ts[next + 1..].join(" ");
    e
}
#[derive(Clone, Copy)]
struct Spec {
    trigger: &'static str,
    dimension: &'static str,
    name: &'static str,
    headers: usize,
    offset: usize,
    kind: u32,
    label: bool,
}
fn specs() -> Vec<Spec> {
    let mut v = Vec::new();
    for (t, d, n, h, k) in [
        ("HtFract", "NTwInpSt", "TowProp", 2, 1),
        ("TwrElev", "NumTwrNds", "TowProp", 2, 1),
        ("BlFract", "NBlInpSt", "BldProp", 2, 1),
        ("Genspd_TLU", "DLL_NumTrq", "DLLProp", 2, 1),
        ("BlSpn", "NumBlNds", "BldAeroNodes", 2, 1),
        ("HvCoefID", "NHvCoef", "HvCoefs", 2, 1),
        ("AxCoefID", "NAxCoef", "AxCoefs", 2, 1),
        ("JointID", "NJoints", "Joints", 2, 1),
        ("Dpth", "NCoefDpth", "DpthProp", 2, 1),
        ("FillNumM", "NFillGroups", "FillGroups", 2, 1),
        ("MGDpth", "NMGDepths", "MGProp", 2, 1),
        ("SimplCd", "1", "SmplProp", 2, 1),
        ("RNodes", "BldNodes", "BldAeroNodes", 1, 6),
        ("kp_xr", "kp_total", "MemberGeom", 2, 1),
        ("mu1", "1", "DampingCoeffs", 2, 1),
        ("TwrHtFr", "NTwrHt", "TowerProp", 1, 1),
        ("TwrRe", "NTwrRe", "TowerRe", 1, 1),
        ("WT_X", "NumTurbines", "WindTurbines", 2, 6),
        ("RJointID", "NReact", "BaseJoints", 2, 6),
        ("IJointID", "NInterf", "InterfaceJoints", 2, 1),
        ("COSMID", "NCOSMs", "MemberCosineMatrix", 2, 1),
        ("CMJointID", "NCmass", "ConcentratedMasses", 2, 1),
        ("WndSpeed", "NumCases", "Cases", 2, 1),
        ("HWndSpeed", "NumCases", "Cases", 2, 1),
    ] {
        v.push(Spec {
            trigger: t,
            dimension: d,
            name: n,
            headers: h,
            offset: 0,
            kind: k,
            label: false,
        });
    }
    for (t, d, n, h, o, k) in [
        ("NumAlf", "NumAlf", "AFCoeff", 2, 0, 2),
        ("F_X", "NKInpSt", "TMDspProp", 2, 0, 1),
        ("MemberCd1", "NCoefMembers", "MemberProp", 2, 0, 1),
        ("MJointID1", "NMembers", "Members", 2, 0, 6),
        ("NOutLoc", "NMOutputs", "MemberOuts", 2, 0, 1),
        ("NOutCnt", "NMOutputs", "MemberOuts", 2, 0, 7),
        ("PropD", "NPropSets", "SectionProp", 2, 0, 1),
        ("Diam", "NTypes:AUTO", "LineTypes", 2, 0, 6),
        ("Type", "NConnects", "ConnectionProp", 2, 0, 6),
        ("LineType", "NLines:AUTO", "LineProp", 2, 0, 6),
        ("Attachment", "AUTO", "Points", 2, 0, 6),
        ("GuyanDampSize", "6", "GuyanDampMatrix", 0, 1, 1),
        ("YoungE", "NPropSets", "BeamProp", 2, 0, 1),
        ("YoungE", "NXPropSets", "BeamPropX", 2, 0, 1),
        ("EA", "NCablePropSets", "CableProp", 2, 0, 1),
        ("MatDens", "NRigidPropSets", "RigidProp", 2, 0, 1),
        ("GridName", "nGridOut", "GridOutputs", 0, 2, 6),
    ] {
        v.push(Spec {
            trigger: t,
            dimension: d,
            name: n,
            headers: h,
            offset: o,
            kind: k,
            label: true,
        });
    }
    v
}
fn cleaned(s: &str) -> &str {
    let s = s.trim().trim_start_matches(['!', '#']).trim();
    s.split(['!', '#']).next().unwrap_or("").trim()
}
fn units(s: &str) -> Vec<String> {
    let s = cleaned(s);
    let mut result = Vec::new();
    let mut current = String::new();
    let mut inside = false;
    for c in s.chars() {
        if c == '(' || c == '[' {
            inside = true;
            current.clear();
        } else if c == ')' || c == ']' {
            if inside {
                result.push(format!("({})", current.trim()));
                inside = false;
            }
        } else if inside {
            current.push(c);
        }
    }
    if result.is_empty() {
        result = s
            .split_whitespace()
            .map(|v| format!("({})", v.trim_matches(['(', ')', '[', ']'])))
            .collect();
    }
    result
}
fn table(
    lines: &[&str],
    start: usize,
    n: usize,
    headers: usize,
    offset: usize,
    kind: u32,
) -> Result<InputTable, IoError> {
    let data_start = start + headers + offset;
    if data_start + n > lines.len() {
        return Err(broken(format!(
            "not enough lines in table at {}",
            start + 1
        )));
    }
    let mut columns = if headers > 0 {
        cleaned(lines[start])
            .split(['(', '['])
            .next()
            .unwrap_or("")
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let mut us = if headers > 1 {
        units(lines[start + 1])
    } else {
        Vec::new()
    };
    let mut rows = Vec::with_capacity(n);
    for (j, line) in lines[data_start..data_start + n].iter().enumerate() {
        if line.trim_start().starts_with("---") {
            return Err(broken(format!(
                "unexpected end of table at {}",
                data_start + j + 1
            )));
        }
        let mut row: Vec<_> = tokens(cleaned(line)).iter().map(|s| atom(s)).collect();
        if kind != 6 && row.iter().any(|v| v.as_f64().is_none()) {
            return Err(broken(format!(
                "non-numeric table at {}",
                data_start + j + 1
            )));
        }
        if j == 0 {
            while columns.len() < row.len() {
                columns.push(if headers == 0 {
                    format!("C{}", columns.len())
                } else {
                    "Col".into()
                });
                if headers > 1 {
                    us.push("(Col)".into());
                }
            }
            if kind != 7 {
                columns.truncate(row.len());
                us.truncate(row.len());
            }
        }
        if kind != 7 {
            if kind == 6 {
                row.resize(columns.len(), InputValue::Integer(0));
                row.truncate(columns.len());
            } else if row.len() != columns.len() {
                return Err(broken("ragged numeric table"));
            }
        }
        rows.push(row);
    }
    if headers > 1 && us.len() != columns.len() {
        return Err(broken("table column/unit count mismatch"));
    }
    Ok(InputTable {
        columns,
        units: us,
        rows,
    })
}
impl FastInputFile {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        let text = read_text(path)?;
        Self::parse(path, &text)
    }
    pub fn parse(path: &Path, text: &str) -> Result<Self, IoError> {
        let lines: Vec<_> = text.lines().collect();
        let mut f = Self {
            filename: path.into(),
            module: None,
            entries: Vec::new(),
            has_nodal: false,
        };
        if f.read_ext_platform(&lines)? || f.read_ad14(&lines)? {
            return Ok(f);
        }
        let mut spec = specs();
        let mut i = 0;
        let mut suffix = String::new();
        let mut polar = 0;
        while i < lines.len() {
            let upper = lines[i].to_ascii_uppercase();
            if upper.contains("DISTRIBUTED PROPERTIES") {
                f.entries.push(comment(lines[i]));
                f.read_beam(&lines, i + 1)?;
                break;
            }
            if upper.contains("ADDITIONAL STIFFNESS") {
                f.entries.push(comment(lines[i]));
                if i + 20 > lines.len() {
                    return Err(broken("truncated additional stiffness"));
                }
                let mut e = entry(
                    "KDAdd",
                    InputValue::List(
                        lines[i + 1..i + 20]
                            .iter()
                            .map(|s| InputValue::Text((*s).into()))
                            .collect(),
                    ),
                );
                e.table_type = 3;
                f.entries.push(e);
                i += 20;
                continue;
            }
            if upper.contains("GLBDCM") {
                f.entries.push(comment(lines[i]));
                let t = table(&lines, i + 2, 3, 0, 0, 1)?;
                let mut e = entry("GlbDCM", InputValue::Table(t));
                e.table_type = 4;
                f.entries.push(e);
                i += 5;
                continue;
            }
            let is_out = upper.contains("ADDITIONAL OUTPUTS")
                || upper.contains("MESH-BASED OUTPUTS")
                || upper.contains("OUTPUT CHANNELS")
                || lines[i].trim_start().starts_with("OutList")
                || upper.contains("SSOUTLIST")
                || upper.contains("SDOUTLIST")
                || (upper.contains("OUTPUTS")
                    && f.get("Points").is_some()
                    && f.get("dtM").is_some());
            if is_out {
                let label = tokens(lines[i])
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "OutList".into());
                let mut vals = vec![InputValue::Text(String::new())];
                i += 1;
                while i < lines.len()
                    && !lines[i]
                        .trim_start()
                        .to_ascii_uppercase()
                        .starts_with("END")
                {
                    vals.push(InputValue::Text(lines[i].trim().into()));
                    i += 1;
                }
                let mut e = entry(
                    if f.has_nodal && !label.ends_with("_Nodal") {
                        format!("{label}_Nodal")
                    } else {
                        label
                    },
                    InputValue::List(vals),
                );
                e.table_type = 33;
                f.entries.push(e);
                f.has_nodal = lines.get(i + 2).is_some_and(|l| {
                    let l = l.to_lowercase();
                    l.contains("bldnd_bladesout") || l.contains("bldnd_bloutnd")
                });
                continue;
            }
            if i == 1
                && ["--", "!", "#", "=="]
                    .iter()
                    .any(|p| lines[0].trim_start().starts_with(*p))
            {
                f.entries.push(comment(lines[i]));
                i += 1;
                continue;
            }
            let mut e = parse_line(lines[i], false);
            let raw = e.label.clone();
            if f.entries
                .last()
                .is_some_and(|e| e.label.eq_ignore_ascii_case("kp_total"))
            {
                let values: Vec<_> = tokens(cleaned(lines[i])).iter().map(|s| atom(s)).collect();
                if values.iter().all(|v| matches!(v, InputValue::Integer(_))) {
                    f.entries
                        .push(entry("kp_per_member", InputValue::List(values)));
                    i += 1;
                    continue;
                }
            }
            if raw.eq_ignore_ascii_case("Re") && f.integer("NumTabs").unwrap_or(1) > 1 {
                polar += 1;
                suffix = format!("_{polar}");
            }
            if !e.is_comment {
                e.label.push_str(&suffix);
            }
            if raw.eq_ignore_ascii_case("NumCoords") && e.value.as_f64().is_some_and(|n| n > 0.0) {
                let n = e.value.as_f64().unwrap() as usize;
                f.entries.push(e);
                if i + 7 >= lines.len() {
                    return Err(broken("truncated airfoil coordinates"));
                }
                for line in &lines[i + 1..i + 4] {
                    f.entries.push(comment(line));
                }
                let p = tokens(cleaned(lines[i + 4]))
                    .iter()
                    .map(|s| atom(s))
                    .collect();
                f.entries
                    .push(entry("AirfoilRefPoint", InputValue::List(p)));
                for line in &lines[i + 5..i + 7] {
                    f.entries.push(comment(line));
                }
                let mut t = table(&lines, i + 7, n - 1, 1, 0, 1)?;
                t.units = vec!["(-)".into(); t.columns.len()];
                let mut e = entry("AirfoilCoord", InputValue::Table(t));
                e.table_type = 2;
                e.dimension = Some("NumCoords".into());
                f.entries.push(e);
                break;
            }
            if raw.eq_ignore_ascii_case("FoilNm") || raw.eq_ignore_ascii_case("AFNames") {
                let dim = if raw.eq_ignore_ascii_case("FoilNm") {
                    "NumFoil"
                } else {
                    "NumAFfiles"
                };
                let n = f
                    .integer(dim)
                    .ok_or_else(|| broken(format!("missing {dim}")))?
                    as usize;
                if i + n > lines.len() {
                    return Err(broken("truncated airfoil filenames"));
                }
                e.value = InputValue::List(
                    lines[i..i + n]
                        .iter()
                        .map(|l| InputValue::Text(tokens(l).first().cloned().unwrap_or_default()))
                        .collect(),
                );
                e.table_type = 3;
                e.dimension = Some(dim.into());
                f.entries.push(e);
                i += n.max(1);
                continue;
            }
            let found = spec.iter().position(|s| {
                if s.label {
                    raw.eq_ignore_ascii_case(s.trigger)
                } else {
                    e.value
                        .as_str()
                        .is_some_and(|v| v.eq_ignore_ascii_case(s.trigger))
                }
            });
            if let Some(si) = found {
                let s = spec[si];
                if s.trigger == "NumAlf" {
                    f.entries.push(e.clone());
                    i += 1;
                }
                if s.offset > 0 {
                    f.entries.push(e.clone());
                    for l in &lines[i + 1..(i + s.offset).min(lines.len())] {
                        f.entries.push(comment(l));
                    }
                }
                let mut count = None;
                for d in s.dimension.split(':') {
                    if d == "AUTO" {
                        let start = i + s.headers + s.offset;
                        if start > lines.len() {
                            return Err(broken("truncated automatic table header"));
                        }
                        count = Some(
                            lines[start..]
                                .iter()
                                .take_while(|l| {
                                    !["---", "!", "#"]
                                        .iter()
                                        .any(|p| l.trim_start().starts_with(*p))
                                        && !l.trim().is_empty()
                                })
                                .count(),
                        );
                        break;
                    }
                    if let Ok(n) = d.parse::<usize>() {
                        count = Some(n);
                        break;
                    }
                    if let Some(n) = f.integer(&format!("{d}{suffix}")).or_else(|| f.integer(d)) {
                        if n < 0 {
                            return Err(broken("negative table count"));
                        }
                        count = Some(n as usize);
                        break;
                    }
                }
                let n = count
                    .ok_or_else(|| broken(format!("missing table dimension {}", s.dimension)))?;
                let t = table(&lines, i, n, s.headers, s.offset, s.kind)?;
                e.label = format!("{}{suffix}", s.name);
                e.value = InputValue::Table(t);
                e.table_type = s.kind;
                e.dimension = Some(
                    if s.dimension.parse::<usize>().is_ok() || s.dimension.contains("AUTO") {
                        s.dimension.into()
                    } else {
                        format!("{}{suffix}", s.dimension)
                    },
                );
                f.entries.push(e);
                i += n + s.headers + s.offset;
                if s.trigger != "NumAlf" {
                    spec.remove(si);
                }
                continue;
            }
            if !e.is_comment
                && i > 2
                && !e
                    .label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_()-\"'".contains(c))
            {
                return Err(IoError::WrongFormat(format!(
                    "invalid FAST label {}",
                    e.label
                )));
            }
            f.entries.push(e);
            i += 1;
        }
        let values = f.entries.iter().filter(|e| !e.is_comment).count();
        if values == 0
            || f.entries.iter().filter(|e| e.is_comment).count() as f64 > lines.len() as f64 * 0.7
        {
            return Err(IoError::WrongFormat("not a FAST input file".into()));
        }
        f.module = if f.get("NumBlNds").is_some() {
            Some("ADBlade")
        } else if f.get("rhoinf").is_some() {
            Some("BD")
        } else if f.get("NBlInpSt").is_some() {
            Some("EDBlade")
        } else if f.get("NTwInpSt").is_some() {
            Some("EDTower")
        } else if f.get("NumCoords").is_some() && f.get("InterpOrd").is_some() {
            Some("ADPolar")
        } else {
            None
        }
        .map(str::to_string);
        Ok(f)
    }
    pub fn get(&self, key: &str) -> Option<&InputValue> {
        self.entries
            .iter()
            .find(|e| !e.is_comment && e.label.eq_ignore_ascii_case(key))
            .map(|e| &e.value)
    }
    pub fn number(&self, key: &str) -> Option<f64> {
        self.get(key)?.as_f64()
    }
    pub fn integer(&self, key: &str) -> Option<i64> {
        let v = self.number(key)?;
        if v.is_finite() && v.fract() == 0.0 {
            Some(v as i64)
        } else {
            None
        }
    }
    pub fn string(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }
    pub fn table(&self, key: &str) -> Option<&InputTable> {
        if let InputValue::Table(t) = self.get(key)? {
            Some(t)
        } else {
            None
        }
    }
    pub fn numbers(&self, key: &str) -> Option<Vec<f64>> {
        match self.get(key)? {
            InputValue::List(v) => v.iter().map(InputValue::as_f64).collect(),
            v => Some(vec![v.as_f64()?]),
        }
    }
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|e| !e.is_comment)
            .map(|e| e.label.as_str())
    }
    pub fn resolve_reference(&self, key: &str) -> Option<PathBuf> {
        let s = self.string(key)?;
        if unused(s) {
            return None;
        }
        Some(reference_path(&self.filename, s.trim_start_matches('@')))
    }
    pub fn set(&mut self, key: &str, value: InputValue) -> Result<(), IoError> {
        let ids: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_comment && e.label.eq_ignore_ascii_case(key))
            .map(|(i, _)| i)
            .collect();
        if ids.is_empty() {
            return Err(broken(format!("missing input key {key}")));
        }
        for i in ids {
            if let InputValue::Table(t) = &value {
                if let Some(d) = self.entries[i].dimension.clone() {
                    if let Some(e) = self
                        .entries
                        .iter_mut()
                        .find(|e| e.label.eq_ignore_ascii_case(&d))
                    {
                        e.value = InputValue::Integer(
                            (t.rows.len() + usize::from(key.eq_ignore_ascii_case("AirfoilCoord")))
                                as i64,
                        );
                    }
                }
            }
            self.entries[i].value = value.clone();
        }
        Ok(())
    }
    pub fn write(&self, path: &Path) -> Result<(), IoError> {
        fs::write(path, self.to_text()?)?;
        Ok(())
    }
    pub fn to_text(&self) -> Result<String, IoError> {
        if self.module.as_deref() == Some("ExtPtfm") {
            return self.ext_text();
        }
        let mut out = String::new();
        for e in &self.entries {
            if e.is_comment {
                out.push_str(&e.value.to_string());
                out.push('\n');
                continue;
            }
            let label = if self.integer("NumTabs").unwrap_or(1) > 1 {
                e.label
                    .rsplit_once('_')
                    .filter(|(_, n)| n.parse::<usize>().is_ok())
                    .map(|(s, _)| s)
                    .unwrap_or(&e.label)
            } else {
                &e.label
            };
            if let InputValue::Table(t) = &e.value {
                if label == "BeamProperties" {
                    out.push_str(&beam_text(t)?);
                    continue;
                }
                if e.table_type != 4
                    && e.dimension.as_deref() != Some("6")
                    && label != "GridOutputs"
                {
                    let prefix = if e.table_type == 2 { "! " } else { "" };
                    out.push_str(prefix);
                    out.push_str(&t.columns.join(" "));
                    out.push('\n');
                    if !t.units.is_empty() && label != "AirfoilCoord" {
                        out.push_str(prefix);
                        out.push_str(&t.units.join(" "));
                        out.push('\n');
                    }
                }
                for r in &t.rows {
                    out.push_str(
                        &r.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                    out.push('\n');
                }
            } else if e.table_type == 33 {
                out.push_str(&format!("{label} {}\n", e.description));
                if let InputValue::List(v) = &e.value {
                    for x in v {
                        if !x.to_string().is_empty() {
                            out.push_str(&x.to_string());
                            out.push('\n');
                        }
                    }
                }
            } else if e.table_type == 3 {
                if let InputValue::List(v) = &e.value {
                    for (i, x) in v.iter().enumerate() {
                        out.push_str(&x.to_string());
                        if i == 0 && label != "KDAdd" {
                            out.push_str(&format!(" {label} {}", e.description));
                        }
                        out.push('\n');
                    }
                }
            } else if label == "AirfoilRefPoint" || label == "kp_per_member" {
                if let InputValue::List(v) = &e.value {
                    out.push_str(
                        &v.iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                    out.push('\n');
                } else {
                    return Err(broken(format!("invalid {label} list")));
                }
            } else {
                out.push_str(&format!("{} {label} {}\n", e.value, e.description));
            }
        }
        Ok(out)
    }
}
pub(crate) fn unused(s: &str) -> bool {
    matches!(
        s.trim_matches(['"', '\'']).to_ascii_lowercase().as_str(),
        "unused" | "nan" | "na" | "none" | ""
    )
}
pub(crate) fn reference_path(file: &Path, s: &str) -> PathBuf {
    let p = PathBuf::from(s.trim_matches(['"', '\'']).replace('\\', "/"));
    if p.is_absolute() {
        p
    } else {
        file.parent().unwrap_or(Path::new(".")).join(p)
    }
}
fn beam_text(t: &InputTable) -> Result<String, IoError> {
    let mut out = String::new();
    for r in &t.rows {
        if r.len() != 73 {
            return Err(broken("invalid BeamDyn properties"));
        }
        out.push_str(&format!("{}\n", r[0]));
        for prefix in ["K", "M"] {
            for a in 1..=6 {
                for b in 1..=6 {
                    let j = t
                        .columns
                        .iter()
                        .position(|c| c == &format!("{prefix}{a}{b}"))
                        .ok_or_else(|| broken("missing BeamDyn matrix entry"))?;
                    out.push_str(&format!("{} ", r[j]));
                }
                out.push('\n');
            }
            out.push('\n');
        }
    }
    Ok(out)
}
impl FileObject for FastInputFile {
    fn format_name(&self) -> &str {
        "FAST input file"
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        self.frames()
    }
    fn info(&self) -> String {
        format!(
            "FAST input file: {}\n{}",
            self.filename.display(),
            self.keys().collect::<Vec<_>>().join(", ")
        )
    }
}
