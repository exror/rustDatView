//! Python export using upstream numerical routines, replayed comparisons and plot styles.
use crate::{ActionKind, CompareMethod, CompareMode, Pipeline, PlotType, Plugin};
use dv_core::{ColumnData, DvError, Result, Table};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptOptions {
    pub lib_flavor: String,
    pub dfs_flavor: String,
    pub one_tab_per_file: bool,
    pub one_raw_tab: bool,
    pub indent: String,
    pub verbose_comment_level: usize,
    pub grid: bool,
    pub log_x: bool,
    pub log_y: bool,
    pub line_styles: Vec<String>,
    pub markers: Vec<String>,
    pub legend_position: String,
    pub line_width: f64,
    pub marker_size: f64,
    pub subplot_rows: usize,
    pub subplot_cols: usize,
    pub show: bool,
    /// Optional flattened subplot index for each selection.
    pub selection_axes: Vec<usize>,
    pub title: String,
    pub x_label: String,
    pub y_label: String,
    pub x_limits: Option<(f64, f64)>,
    pub y_limits: Option<(f64, f64)>,
    pub flip_x: bool,
    pub flip_y: bool,
    pub legend: bool,
    pub step: bool,
}
impl Default for ScriptOptions {
    fn default() -> Self {
        Self {
            lib_flavor: "pydatview".into(),
            dfs_flavor: "dict".into(),
            one_tab_per_file: false,
            one_raw_tab: false,
            indent: "    ".into(),
            verbose_comment_level: 2,
            grid: false,
            log_x: false,
            log_y: false,
            line_styles: vec!["-".into(), "--".into(), "-.".into(), ":".into()],
            markers: vec![String::new()],
            legend_position: "best".into(),
            line_width: 1.5,
            marker_size: 2.0,
            subplot_rows: 1,
            subplot_cols: 1,
            show: true,
            selection_axes: Vec::new(),
            title: String::new(),
            x_label: String::new(),
            y_label: String::new(),
            x_limits: None,
            y_limits: None,
            flip_x: false,
            flip_y: false,
            legend: true,
            step: false,
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PythonScripter {
    pub files: Vec<String>,
    pub selections: Vec<(usize, usize, usize)>,
    pub options: ScriptOptions,
    pub plot_type: PlotType,
    #[serde(default)]
    pub comparison: Option<(CompareMode, CompareMethod)>,
}
pub(crate) fn quote(s: &str) -> String {
    serde_json::to_string(s).expect("string serialization")
}
fn number(v: f64) -> String {
    if v.is_nan() {
        "np.nan".into()
    } else if v == f64::INFINITY {
        "np.inf".into()
    } else if v == f64::NEG_INFINITY {
        "-np.inf".into()
    } else {
        v.to_string()
    }
}
fn numbers(v: &[f64]) -> String {
    format!(
        "[{}]",
        v.iter().map(|&n| number(n)).collect::<Vec<_>>().join(", ")
    )
}
fn strings(v: &[String]) -> String {
    format!(
        "[{}]",
        v.iter().map(|s| quote(s)).collect::<Vec<_>>().join(", ")
    )
}
fn boolean(v: bool) -> &'static str {
    if v {
        "True"
    } else {
        "False"
    }
}
fn dataframe_literal(t: &Table) -> String {
    // Positional construction preserves duplicate column names.
    let cols = t
        .data
        .columns()
        .iter()
        .map(|c| match &c.data {
            ColumnData::Float(v) => format!("pd.Series({}, dtype=float)", numbers(v)),
            ColumnData::Str(v) => format!("pd.Series({}, dtype=str)", strings(v)),
            ColumnData::DateTime(v) => {
                format!("pd.Series(pd.to_datetime({}, unit='s'))", numbers(v))
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    if cols.is_empty() {
        return "pd.DataFrame()".into();
    }
    format!(
        "pd.concat([{cols}], axis=1).set_axis({}, axis=1)",
        strings(&t.columns())
    )
}
fn formula_code(table: &Table, expression: &str) -> Result<String> {
    let re = regex::Regex::new(r"\{([^}]+)\}").map_err(|e| DvError::msg(e.to_string()))?;
    let mut missing = None;
    let code = re
        .replace_all(expression, |c: &regex::Captures<'_>| {
            let found = table.data.columns().iter().position(|col| {
                dv_core::common::no_unit(&col.name).trim() == c[1].trim() || col.name == c[1]
            });
            if let Some(i) = found {
                format!("np.asarray(df.iloc[:, {i}])")
            } else {
                missing = Some(c[1].to_string());
                String::new()
            }
        })
        .into_owned();
    if let Some(name) = missing {
        return Err(DvError::msg(format!("Unknown formula column {name}")));
    }
    Ok(format!("evaluate_expression(df, {})", quote(&code)))
}
fn mask_code(expression: &str) -> String {
    // Resolve curly-brace names against each dataframe, after any earlier renames.
    if expression.trim().is_empty() || expression.trim().eq_ignore_ascii_case("no mask") {
        return "None".into();
    }
    format!("evaluate_mask(df, {})", quote(expression))
}
fn plot_plugin_code(p: &Plugin) -> Result<String> {
    Ok(match p {
        Plugin::Filter { name, param } => format!(
            "y = applyFilter(x, y, {{'name': {}, 'param': {}}})",
            quote(name),
            number(*param)
        ),
        Plugin::Sampler { name, param } => format!(
            "x, y = applySampler(x, y, {{'name': {}, 'param': {}}})",
            quote(name),
            numbers(param)
        ),
        Plugin::Binning { bins, min, max } => format!(
            "x, y = bin_signal(x, y, xbins=np.linspace({}, {}, {}))",
            number(*min),
            number(*max),
            bins + 1
        ),
        Plugin::Outliers { median_deviation } => format!(
            "x, y = reject_outliers(y, x, m={}, replaceNaN=True)",
            number(*median_deviation)
        ),
        _ => return Err(DvError::msg("Table plugin cannot run on plot data")),
    })
}
fn adder_code(plugin: &Plugin, x_column: usize, indent: &str) -> Result<String> {
    Ok(match plugin{
        Plugin::RadialAverage{method,param}=>format!("dfs_new, names_new = radialAvg(filename, df=df, avgMethod={}, avgParam={}, raiseException=True)\nif not any(d is not None for d in dfs_new):\n{indent}raise ValueError('No OpenFAST radial data found')\n_added.extend(d for d in dfs_new if d is not None)",quote(method),number(*param)),
        Plugin::RadialConcat=>"_added.append(spanwiseConcat(df))".into(),
        Plugin::Mask{expression}=>if mask_code(expression)=="None"{"_added.append(df.copy())".into()}else{format!("_added.append(df.loc[{}].copy())",mask_code(expression))},
        Plugin::Binning{bins,min,max}=>format!("df_new = bin_DF(df, xbins=np.linspace({}, {}, {}), colBin=df.columns[{x_column}])\norder = [{x_column}] + [j for j in range(df_new.shape[1]) if j != {x_column} and str(df_new.columns[j]).lower() != 'index']\n_added.append(df_new.iloc[:, order])",number(*min),number(*max),bins+1),
        p if p.is_plot()=>{
            let xy=plot_plugin_code(p)?;
            let preserve=if matches!(p,Plugin::Sampler{..}){String::new()}else{format!("{indent}if j == {x_column}:\n{indent}{indent}columns.append(df.iloc[:, j].reset_index(drop=True))\n{indent}{indent}continue\n")};
            format!("columns = []\nx_original = np.asarray(df.iloc[:, {x_column}])\nif x_original.dtype.kind == 'M':\n{indent}x_original = x_original.astype('datetime64[ns]').astype(np.int64) / 1e9\nfor j in range(df.shape[1]):\n{indent}if str(df.columns[j]).lower() == 'index':\n{indent}{indent}continue\n{preserve}{indent}x, y = x_original.copy(), np.asarray(df.iloc[:, j]).copy()\n{}\n{indent}columns.append(pd.Series(x if j == {x_column} else y, name=df.columns[j]))\ndf_new = pd.concat(columns, axis=1)\n_added.append(df_new)",xy.lines().map(|l|format!("{indent}{l}")).collect::<Vec<_>>().join("\n"))
        },
        _=>return Err(DvError::msg("Plugin cannot generate tables")),
    })
}

fn append_added(lines: &mut Vec<String>, indent: &str) {
    lines.push("for df in _added:".into());
    lines.push(format!("{indent}if len(df.columns) == 0 or 'index' not in str(df.columns[0]).lower():\n{indent}{indent}df.insert(0, 'Index', np.arange(len(df)))\n{indent}_dfs.append(df)\n{indent}_sources.append('')\n{indent}_masks.append(None)"));
}
fn replay_addition(
    lines: &mut Vec<String>,
    addition: &crate::pipeline::Addition,
    indent: &str,
) -> Result<()> {
    let input_list = format!(
        "[{}]",
        addition
            .inputs
            .iter()
            .map(|id| format!("_indices[{id}]"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    lines.push("_added = []".into());
    if let Plugin::Concat { common_only } = addition.plugin {
        lines.push(format!("_added.append(pd.concat([_dfs[i].iloc[:, 1:] if str(_dfs[i].columns[0]).lower() == 'index' else _dfs[i] for i in {input_list}], ignore_index=True, join={}))",quote(if common_only{"inner"}else{"outer"})));
    } else {
        lines.push(format!(
            "for i in {input_list}:\n{indent}df = _dfs[i]\n{indent}filename = _sources[i]"
        ));
        for l in adder_code(&addition.plugin, addition.x_column, indent)?.lines() {
            lines.push(format!("{indent}{l}"));
        }
    }
    lines.push(format!("if len(_added) != {}:\n{indent}raise ValueError('Generated table count changed while replaying the action')",addition.outputs.len()));
    for (offset, id) in addition.outputs.iter().enumerate() {
        lines.push(format!("_indices[{id}] = len(_dfs) + {offset}"));
    }
    append_added(lines, indent);
    Ok(())
}
fn transform_code(kind: &PlotType) -> Result<String> {
    Ok(match kind {
        PlotType::Regular => String::new(),
        PlotType::Pdf(o) => {
            let numeric = if o.smooth {
                format!("x, y = pdf_histogram(y, nBins=bins, norm=True, count=False) if np.nanmax(y) == np.nanmin(y) else pdf_gaussian_kde(y, nOut=bins)")
            } else {
                "x, y = pdf_histogram(y, nBins=bins, norm=True, count=False)".into()
            };
            format!("if y.dtype.kind == 'M':\n    raise ValueError('Cannot plot PDF of dates')\nelif y.dtype.kind in 'OUS':\n    if len(y) > 100:\n        raise ValueError('String PDF dataset is too large')\n    counts = pd.Series(y).value_counts().sort_index()\n    x, y = counts.index.to_numpy(), counts.to_numpy() / len(y)\nelse:\n    bins = min({}, len(y))\n    {numeric}",o.bins)
        }
        PlotType::MinMax(o) => {
            let mut lines = Vec::new();
            if o.y_center == "None" {
                if o.y_scale {
                    lines.push("y = np.zeros_like(y, dtype=float) if np.nanmax(y) == np.nanmin(y) else (y - np.nanmin(y)) / (np.nanmax(y) - np.nanmin(y))".into());
                }
            } else {
                let center = match o.y_center.as_str() {
                    "Mean=0" | "Mean=ref" => "np.nanmean(y)",
                    "Mid=0" | "Mid=ref" => "(np.nanmax(y) + np.nanmin(y)) / 2",
                    _ => return Err(DvError::msg("Unsupported min-max centering")),
                };
                let reference = if o.y_center.ends_with("=0") {
                    0.0
                } else {
                    o.y_ref
                };
                lines.push(format!("y = y + {} - ({center})", number(reference)));
            }
            if o.x_scale {
                lines.push("x = np.zeros_like(x, dtype=float) if np.nanmax(x) == np.nanmin(x) else (x - np.nanmin(x)) / (np.nanmax(x) - np.nanmin(x))".into());
            }
            lines.join("\n")
        }
        PlotType::Fft(o) => {
            let mut code=format!("x, y, Info = fft_wrap(x, y, output_type={}, averaging={}, averaging_window={}, detrend={}, nExp={}, nPerDecade={})",quote(&o.y_type),quote(&o.averaging),quote(&o.window),boolean(o.detrend),o.n_exp,o.points_per_decade);
            match o.x_type.as_str() {
                "1/x" => {}
                "x" => code.push_str("\nx = 1 / x"),
                "2pi/x" => code.push_str("\nx = 2 * np.pi * x"),
                _ => return Err(DvError::msg("Unsupported FFT x type")),
            }
            code
        }
        PlotType::Polar(o) => {
            let mut lines = vec![
                format!("theta = x{}", if o.degrees { " * np.pi / 180" } else { "" }),
                "radius = y.copy()".into(),
            ];
            if let Some(r) = o.radius_ref {
                lines.push(format!("radius = radius - np.mean(radius) + {}", number(r)));
            }
            if let Some(n) = o.bins {
                lines.push(format!("theta, radius = bin_signal(theta, radius, xbins=np.linspace(0, 2 * np.pi, {n}))"));
                lines.push(
                    "theta = np.r_[theta, theta[0]]\nradius = np.r_[radius, radius[0]]".into(),
                );
            }
            if o.about.starts_with('x') {
                lines.push("x, y = -radius * np.sin(theta), radius * np.cos(theta)".into());
            } else if o.about.starts_with('z') {
                lines.push("x, y = radius * np.cos(theta), radius * np.sin(theta)".into());
            } else {
                return Err(DvError::msg("Polar axis must be x or z"));
            }
            lines.join("\n")
        }
    })
}
fn plot_labels(sx: &str, sy: &str, kind: &PlotType) -> (String, String) {
    use dv_core::common::{inverse_unit, no_unit, splitunit, unit};
    let sx = sx.replace('_', " ");
    let sy = sy.replace('_', " ");
    match kind {
        PlotType::Pdf(_) => {
            let inv = inverse_unit(&sy);
            (
                sy.clone(),
                format!(
                    "PDF({}){}",
                    no_unit(&sy),
                    if inv.is_empty() {
                        String::new()
                    } else {
                        format!(" [{inv}]")
                    }
                ),
            )
        }
        PlotType::Fft(o) => {
            let (name, unt) = splitunit(&sy);
            let xu = unit(&sx);
            let y = match o.y_type.as_str() {
                "PSD" => format!("PSD({name}) [({unt})^2/{xu}]"),
                "f x PSD" => format!("f-weighted PSD({name}) [({unt})^2]"),
                _ => format!("FFT({name}) [{unt}]"),
            };
            let x = if xu == "s" {
                match o.x_type.as_str() {
                    "x" => "Period [s]",
                    "2pi/x" => "Cyclic frequency [rad/s]",
                    _ => "Frequency [Hz]",
                }
                .into()
            } else {
                String::new()
            };
            (x, y)
        }
        PlotType::Polar(o) => {
            let (name, unt) = splitunit(&sy);
            let (a, b) = if o.about.starts_with('x') {
                ("y", "z")
            } else {
                ("x", "y")
            };
            (
                format!("{}_{a} [{unt}]", name.trim()),
                format!("{}_{b} [{unt}]", name.trim()),
            )
        }
        _ => (sx, sy),
    }
}

const MASK_CODE: &str = r#"def _truth(value):
    value = np.asarray(value)
    return (value != 0) & ~np.isnan(value)

def _comparison(left, right, operation):
    def date(value):
        dayfirst = '/' in value and len(value.split('/')[0]) != 4
        return np.datetime64(pd.to_datetime(value, dayfirst=dayfirst))
    if np.asarray(left).dtype.kind == 'M' and isinstance(right, str):
        right = date(right)
    if np.asarray(right).dtype.kind == 'M' and isinstance(left, str):
        left = date(left)
    return getattr(np, operation)(left, right)

class _BooleanArrays(ast.NodeTransformer):
    def visit_BoolOp(self, node):
        values = [ast.Call(func=ast.Name(id='_truth', ctx=ast.Load()), args=[self.visit(v)], keywords=[]) for v in node.values]
        result = values[0]
        for value in values[1:]:
            result = ast.Call(func=ast.Attribute(value=ast.Name(id='np', ctx=ast.Load()), attr='logical_and' if isinstance(node.op, ast.And) else 'logical_or', ctx=ast.Load()), args=[result, value], keywords=[])
        return ast.copy_location(result, node)
    def visit_UnaryOp(self, node):
        if isinstance(node.op, ast.Not):
            result = ast.Call(func=ast.Attribute(value=ast.Name(id='np', ctx=ast.Load()), attr='logical_not', ctx=ast.Load()), args=[ast.Call(func=ast.Name(id='_truth', ctx=ast.Load()), args=[self.visit(node.operand)], keywords=[])], keywords=[])
            return ast.copy_location(result, node)
        return self.generic_visit(node)
    def visit_Compare(self, node):
        if any(isinstance(op, (ast.In, ast.NotIn)) for op in node.ops):
            return self.generic_visit(node)
        operations = {ast.Eq: 'equal', ast.NotEq: 'not_equal', ast.Lt: 'less', ast.LtE: 'less_equal', ast.Gt: 'greater', ast.GtE: 'greater_equal'}
        operands = [self.visit(node.left)] + [self.visit(v) for v in node.comparators]
        comparisons = [ast.Call(func=ast.Name(id='_comparison', ctx=ast.Load()), args=[operands[i], operands[i + 1], ast.Constant(value=operations[type(op)])], keywords=[]) for i, op in enumerate(node.ops)]
        result = comparisons[0]
        for comparison in comparisons[1:]:
            result = ast.Call(func=ast.Attribute(value=ast.Name(id='np', ctx=ast.Load()), attr='logical_and', ctx=ast.Load()), args=[result, comparison], keywords=[])
        return ast.copy_location(result, node)

def evaluate_expression(df, expression):
    def column(match):
        name = match[1].strip()
        index = next(i for i, c in enumerate(df.columns) if c == name or no_unit(c).strip() == name)
        return 'np.asarray(df.iloc[:, ' + str(index) + '])'
    expression = re.sub(r'\{([^}]+)\}', column, expression)
    expression = expression.replace('&&', '&').replace('||', '|')
    tokens = list(tokenize.generate_tokens(io.StringIO(expression).readline))
    expression = tokenize.untokenize([(t.type, {'&': ' and ', '|': ' or ', '~': ' not '}.get(t.string, t.string) if t.type == tokenize.OP else t.string) for t in tokens])
    tree = ast.fix_missing_locations(_BooleanArrays().visit(ast.parse(expression.strip(), mode='eval')))
    namespace = dict(np.__dict__, np=np, numpy=np, math=np, ln=np.log, df=df, _truth=_truth, _comparison=_comparison, str=str, int=int, float=float, bool=bool, len=len)
    namespace['__builtins__'] = {}
    return eval(compile(tree, '<expression>', 'eval'), namespace)

def evaluate_mask(df, expression):
    mask = np.broadcast_to(_truth(evaluate_expression(df, expression)), (len(df),)).copy()
    if not np.any(mask):
        raise ValueError('Mask returned no values')
    return mask"#;

const COMPARE_CODE: &str = r#"def compare_curves(curves, mode, method):
    if len(curves) < 2:
        raise ValueError('Comparison requires at least two signals')
    groups = {}
    for curve in curves:
        if curve['y'].dtype.kind in 'MOUS':
            raise ValueError('Cannot compare dates or strings')
        if mode == 'TablesSameColumns':
            key = curve['column']
        elif mode == 'TablesSimilarColumns':
            key = re.sub(r'[_ -]', '', no_unit(curve['sy']).lower().strip())
        else:
            key = 0
        groups.setdefault(key, []).append(curve)
    output = []
    for group in groups.values():
        ref = group[0]
        near_zero = np.mean(np.abs(ref['y'])) < 1e-7
        others = group[1:2] if mode == 'TablesOneColumn' else group[1:]
        for curve in others:
            curve = curve.copy()
            if mode == 'OneTableColumns' or ref['x'].dtype.kind in 'OUS' or curve['x'].dtype.kind in 'OUS':
                if len(curve['y']) != len(ref['y']):
                    raise ValueError('Cannot compare different lengths without numeric x')
                y = curve['y']
            else:
                y = np.interp(ref['x'], curve['x'], curve['y'])
            if method == 'YY':
                curve['x'], curve['y'] = ref['y'], y
                curve['sx'] = ref['sy']
            else:
                curve['x'] = ref['x']
                curve['sx'] = ref['sx']
                denominator = ref['y'] + (1 if near_zero else 0)
                if method == 'Relative':
                    curve['y'] = (y - ref['y']) / denominator * 100
                    curve['sy'] = 'Relative error [%]'
                elif method == 'AbsRelative':
                    curve['y'] = np.abs((y - ref['y']) / denominator) * 100
                    curve['sy'] = 'Abs. relative error [%]'
                elif method == 'Ratio':
                    curve['y'] = (y + (1 if near_zero else 0)) / denominator
                    curve['sy'] = 'Ratio [-]'
                elif method == 'Absolute':
                    curve['y'] = y - ref['y']
                    unit = ref['sy'].rsplit('[', 1)[-1].rstrip(']') if '[' in ref['sy'] else ''
                    curve['sy'] = 'Absolute error [' + unit + ']'
                else:
                    raise ValueError('Unsupported comparison method')
            curve['label'] += ' wrt. ' + ref['label']
            output.append(curve)
    if not output:
        raise ValueError('No matching columns to compare')
    return output"#;
impl PythonScripter {
    pub fn generate(&self, tables: &[Table], pipeline: &Pipeline) -> Result<String> {
        let o = &self.options;
        if !matches!(
            o.lib_flavor.as_str(),
            "pydatview" | "welib" | "openfast_toolbox"
        ) {
            return Err(DvError::msg("Unsupported Python library flavor"));
        }
        if !matches!(o.dfs_flavor.as_str(), "dict" | "list" | "enumeration") {
            return Err(DvError::msg("Unsupported dataframe flavor"));
        }
        if o.subplot_rows == 0
            || o.subplot_cols == 0
            || o.line_styles.is_empty()
            || o.markers.is_empty()
            || o.indent.is_empty()
            || !o.indent.chars().all(|c| c == ' ' || c == '\t')
        {
            return Err(DvError::msg(
                "Invalid plot style, subplot dimensions or indentation",
            ));
        }
        if o.one_raw_tab && tables.iter().filter(|t| !t.filename.is_empty()).count() > 1 {
            return Err(DvError::msg("one_raw_tab requires one input table"));
        }
        let plugins: Vec<&Plugin> = pipeline
            .actions
            .iter()
            .filter(|a| a.enabled)
            .map(|a| &a.plugin)
            .chain(
                pipeline
                    .actions
                    .iter()
                    .flat_map(|a| a.additions.iter().map(|r| &r.plugin)),
            )
            .collect();
        let mut imports = vec![
            "import numpy as np",
            "import pandas as pd",
            "import re",
            "import matplotlib.pyplot as plt",
        ];
        if tables.iter().any(|t| !t.filename.is_empty()) || !self.files.is_empty() {
            imports.push("import pydatview.io as weio");
        }
        if plugins.iter().any(|p| p.is_plot()) {
            imports.push("from pydatview.tools.signal_analysis import applyFilter, applySampler, reject_outliers");
        }
        if plugins.iter().any(|p| matches!(p, Plugin::Binning { .. }))
            || matches!(self.plot_type, PlotType::Pdf(_) | PlotType::Polar(_))
        {
            imports.push("from pydatview.tools.stats import bin_DF, bin_signal, pdf_histogram, pdf_gaussian_kde");
        }
        if matches!(self.plot_type, PlotType::Fft(_)) {
            imports.push("from pydatview.tools.spectral import fft_wrap");
        }
        if plugins.iter().any(|p| matches!(p, Plugin::Units { .. })) {
            imports.push("from pydatview.tools.pandalib import changeUnits");
        }
        if plugins
            .iter()
            .any(|p| matches!(p, Plugin::RadialAverage { .. } | Plugin::RadialConcat))
        {
            imports.push("from pydatview.fast.postpro import radialAvg, spanwiseConcat");
        }
        let expressions = plugins.iter().any(|p| matches!(p, Plugin::Mask { .. }))
            || tables.iter().any(|t| !t.formulas.is_empty());
        if expressions {
            imports.extend(["import ast", "import io", "import tokenize"]);
        }
        imports.sort();
        let mut lines = vec![
            "\"\"\"Generated by rustDatView. Replays the selected files, actions and plots.\"\"\""
                .into(),
        ];
        for import in imports {
            let s = match o.lib_flavor.as_str() {
                "welib" => import
                    .replace("pydatview.io", "welib.weio")
                    .replace("pydatview.tools", "welib.tools")
                    .replace("pydatview.fast.postpro", "welib.fast.postpro"),
                "openfast_toolbox" => import
                    .replace("pydatview.io", "openfast_toolbox.io")
                    .replace("pydatview.tools", "openfast_toolbox.tools")
                    .replace("pydatview.fast.postpro", "openfast_toolbox.postpro"),
                _ => import.to_string(),
            };
            lines.push(s);
        }
        let indent = &o.indent;
        lines.push(format!("def no_unit(s):\n{indent}s = s.replace('(', ' [').replace(')', ']').replace('_[', ' [')\n{indent}i = s.rfind('[')\n{indent}return s[:i].strip() if i > 0 else s"));
        if expressions {
            lines.push(
                MASK_CODE
                    .lines()
                    .map(|l| {
                        let leading = l.len() - l.trim_start_matches(' ').len();
                        format!(
                            "{}{}",
                            indent.repeat(leading / 4),
                            l.trim_start_matches(' ')
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        let mut files = self.files.clone();
        for t in tables {
            if !t.filename.is_empty() && !files.contains(&t.filename) {
                files.push(t.filename.clone());
            }
        }
        let mut seen = HashSet::new();
        files = files
            .into_iter()
            .map(|f| f.replace('\\', "/"))
            .filter(|f| !f.is_empty() && seen.insert(f.clone()))
            .collect();
        lines.push(format!(
            "\nfilenames = {}",
            strings(
                &files
                    .iter()
                    .map(|f| f.replace('\\', "/"))
                    .collect::<Vec<_>>()
            )
        ));
        lines.push("loaded = {}".into());
        if !files.is_empty() {
            lines.push("for filename in filenames:".into());
            lines.push(format!(
                "{indent}loaded[filename] = weio.read(filename).toDataFrame()"
            ));
        }
        if o.one_tab_per_file && !files.is_empty() {
            lines.push(format!("{indent}if isinstance(loaded[filename], dict):\n{indent}{indent}raise ValueError('one_tab_per_file requested for a multi-table file')"));
        }
        lines.push("_dfs = []\n_sources = []\n_masks = []\n_indices = {}".into());
        let table_ids: HashSet<_> = tables.iter().map(Table::identity).collect();
        let replayed: HashSet<_> = pipeline
            .actions
            .iter()
            .flat_map(|a| &a.additions)
            .filter(|r| r.inputs.iter().all(|id| table_ids.contains(id)))
            .flat_map(|r| r.outputs.iter().copied())
            .collect();
        let mut available = HashSet::<u64>::new();
        for t in tables {
            if replayed.contains(&t.identity()) {
                continue;
            }
            available.insert(t.identity());
            lines.push(format!("_indices[{}] = len(_dfs)", t.identity()));
            if t.filename.is_empty() {
                lines.push(format!("_dfs.append({})", dataframe_literal(t)));
            } else {
                let path = t.filename.replace('\\', "/");
                lines.push(format!("raw = loaded[{}]", quote(&path)));
                lines.push(format!(
                    "_dfs.append((raw[{}] if isinstance(raw, dict) else raw).copy())",
                    quote(t.source_frame_name())
                ));
            }
            lines.push(format!(
                "_sources.append({})\n_masks.append(None)",
                quote(&t.filename.replace('\\', "/"))
            ));
        }
        lines.push("for df in _dfs:".into());
        lines.push(format!("{indent}df.columns = [str(c) for c in df.columns]\n{indent}if len(df.columns) == 0 or 'index' not in str(df.columns[0]).lower():\n{indent}{indent}df.insert(0, 'Index', np.arange(len(df)))"));
        let source_tables: Vec<&Table> = tables
            .iter()
            .filter(|t| !replayed.contains(&t.identity()))
            .collect();
        for (i, t) in source_tables.iter().enumerate() {
            if !t.filename.is_empty() {
                for formula in &t.formulas {
                    lines.push(format!("df = _dfs[{i}]\nvalues = {}\nif {} < df.shape[1] and df.columns[{}] == {}:\n{indent}df.iloc[:, {}] = values\nelse:\n{indent}df.insert({}, {}, values, allow_duplicates=True)",formula_code(t,&formula.formula)?,formula.pos,formula.pos,quote(&formula.name),formula.pos,formula.pos,quote(&formula.name)));
                }
            }
        }
        let mut pending = Vec::new();
        for action in &pipeline.actions {
            pending.extend(
                action
                    .additions
                    .iter()
                    .filter(|r| r.outputs.iter().any(|id| replayed.contains(id))),
            );
            while let Some(i) = pending
                .iter()
                .position(|r| r.inputs.iter().all(|id| available.contains(id)))
            {
                let addition = pending.remove(i);
                replay_addition(&mut lines, addition, indent)?;
                available.extend(addition.outputs.iter().copied());
            }
            if !action.enabled
                || action.kind == ActionKind::PlotData
                || (action.kind == ActionKind::Adder && !action.additions.is_empty())
            {
                continue;
            }
            if o.verbose_comment_level > 0 {
                lines.push(format!(
                    "\n# Action: {}",
                    action.name.replace(['\n', '\r'], " ")
                ));
            }
            if let Plugin::Concat { common_only } = action.plugin {
                lines.push(format!("_dfs.append(pd.concat([df.iloc[:, 1:] if str(df.columns[0]).lower() == 'index' else df for df in _dfs], ignore_index=True, join={}))\n_sources.append('')\n_masks.append(None)",quote(if common_only{"inner"}else{"outer"})));
                lines.push("_dfs[-1].insert(0, 'Index', np.arange(len(_dfs[-1])))".into());
                continue;
            }
            lines.push("_added = []\nfor i, df in enumerate(_dfs):".into());
            lines.push(format!("{indent}filename = _sources[i]"));
            if action.kind != ActionKind::Adder && !matches!(action.plugin, Plugin::Mask { .. }) {
                let skip: Vec<f64> = source_tables
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.filename.is_empty() && action.applied_to(t))
                    .map(|(i, _)| i as f64)
                    .collect();
                lines.push(format!(
                    "{indent}if i in {}:\n{indent}{indent}continue",
                    numbers(&skip)
                ));
            }
            let code = if action.kind == ActionKind::Adder {
                adder_code(&action.plugin, action.x_column, indent)?
            } else {
                match &action.plugin{
                Plugin::Mask{expression}=>format!("_masks[i] = {}",mask_code(expression)),
                Plugin::Units{flavor}=>format!("changeUnits(df, flavor={})",quote(flavor)),
                Plugin::RenameChannels{flavor}=>match flavor.as_str(){
                    "OF2To3"=>"df.columns = [re.sub(r'^B(?=\\dN)', 'AB', c).replace('AOA_', 'Alpha_').replace('AIn_', 'AxInd_').replace('ApI_', 'TnInd_') for c in df.columns]".into(),
                    "OF3To2"=>"df.columns = [re.sub(r'^AB(?=\\dN)', 'B', c).replace('Alpha_', 'AOA_').replace('AxInd_', 'AIn_').replace('TnInd_', 'ApI_') for c in df.columns]".into(),
                    "FldToAero"=>"df.columns = [c.replace('Fld', 'Aero') for c in df.columns]".into(),
                    "AeroToFld"=>"df.columns = [c.replace('Aero', 'Fld') for c in df.columns]".into(),
                    _=>return Err(DvError::msg("Unknown channel rename flavor")),
                },
                _=>return Err(DvError::msg(format!("Cannot export table action {}",action.name))),
            }
            };
            for line in code.lines() {
                lines.push(format!("{indent}{line}"));
            }
            append_added(&mut lines, indent);
        }
        if !pending.is_empty() {
            return Err(DvError::msg(
                "Generated table dependencies cannot be replayed",
            ));
        }
        for t in tables.iter().filter(|t| replayed.contains(&t.identity())) {
            for formula in &t.formulas {
                lines.push(format!(
                    "df = _dfs[_indices[{}]]\ndf.insert({}, {}, {}, allow_duplicates=True)",
                    t.identity(),
                    formula.pos,
                    quote(&formula.name),
                    formula_code(t, &formula.formula)?
                ));
            }
        }
        let order = tables
            .iter()
            .map(|t| format!("_indices[{}]", t.identity()))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!("order = [{order}]\n_dfs = [_dfs[i] for i in order]\n_masks = [_masks[i] for i in order]"));
        match o.dfs_flavor.as_str() {
            "dict" => lines.push("dfs = dict(enumerate(_dfs))".into()),
            "list" => lines.push("dfs = _dfs".into()),
            _ => {
                for i in 0..tables.len() {
                    lines.push(format!("df{} = _dfs[{i}]", i + 1));
                }
            }
        }
        lines.push("\ndef preproXY(x, y):".into());
        lines.push(format!(
            "{indent}x, y = np.asarray(x).copy(), np.asarray(y).copy()"
        ));
        if pipeline.has_plot_actions() || matches!(self.plot_type, PlotType::Fft(_)) {
            lines.push(format!("{indent}if x.dtype.kind == 'M':\n{indent}{indent}x = x.astype('datetime64[ns]').astype(np.int64) / 1e9"));
        }
        for action in &pipeline.actions {
            if action.enabled && action.kind == ActionKind::PlotData {
                for l in plot_plugin_code(&action.plugin)?.lines() {
                    lines.push(format!("{indent}{l}"));
                }
            }
        }
        for l in transform_code(&self.plot_type)?.lines() {
            let leading = l.len() - l.trim_start_matches(' ').len();
            let nested = indent.repeat(leading / 4);
            lines.push(format!("{indent}{nested}{}", l.trim_start_matches(' ')));
        }
        lines.push(format!("{indent}return x, y"));
        lines.push(format!("\nfig, axes = plt.subplots({}, {}, squeeze=False, sharex=True, figsize=(6.4, 4.8))\naxes = axes.ravel()",o.subplot_rows,o.subplot_cols));
        if self.comparison.is_some() {
            lines.push(
                COMPARE_CODE
                    .lines()
                    .map(|l| {
                        let leading = l.len() - l.trim_start_matches(' ').len();
                        format!(
                            "{}{}",
                            indent.repeat(leading / 4),
                            l.trim_start_matches(' ')
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
            lines.push("curves = []".into());
        }
        for (i, &(it, ix, iy)) in self.selections.iter().enumerate() {
            let table = tables
                .get(it)
                .ok_or_else(|| DvError::msg("Selected table out of bounds"))?;
            if ix >= table.ncols() || iy >= table.ncols() {
                return Err(DvError::msg("Selected column out of bounds"));
            }
            let axis = o.selection_axes.get(i).copied().unwrap_or(0);
            if axis >= o.subplot_rows * o.subplot_cols {
                return Err(DvError::msg("Selected subplot out of bounds"));
            }
            let (sx, sy) = plot_labels(
                &table.data.column(ix).name,
                &table.data.column(iy).name,
                &self.plot_type,
            );
            lines.push(format!("df = _dfs[{it}]\nif _masks[{it}] is not None:\n{indent}df = df.loc[_masks[{it}]]\nx, y = preproXY(df.iloc[:, {ix}], df.iloc[:, {iy}])\nax = axes[{axis}]"));
            let label = format!("{} | {sy}", table.nickname());
            if self.comparison.is_some() {
                lines.push(format!("curves.append({{'x': x, 'y': y, 'sx': {}, 'sy': {}, 'label': {}, 'column': {iy}, 'axis': {axis}, 'style': {}, 'marker': {}}})",quote(&sx),quote(&sy),quote(&label),quote(&o.line_styles[i%o.line_styles.len()]),quote(&o.markers[i%o.markers.len()])));
            } else {
                lines.push(format!(
                    "ax.{}(x, y, {}ls={}, marker={}, lw={}, ms={}, label={})",
                    if o.step { "step" } else { "plot" },
                    if o.step { "where='post', " } else { "" },
                    quote(&o.line_styles[i % o.line_styles.len()]),
                    quote(&o.markers[i % o.markers.len()]),
                    number(o.line_width),
                    number(o.marker_size),
                    quote(&label)
                ));
                lines.push(format!(
                    "ax.set_xlabel({})\nax.set_ylabel({})",
                    quote(&sx),
                    quote(&sy)
                ));
            }
        }
        if let Some((mode, method)) = self.comparison {
            lines.push(format!("for curve in compare_curves(curves, {}, {}):\n{indent}ax = axes[curve['axis']]\n{indent}ax.{}(curve['x'], curve['y'], {}ls=curve['style'], marker=curve['marker'], lw={}, ms={}, label=curve['label'])\n{indent}ax.set_xlabel(curve['sx'])\n{indent}ax.set_ylabel(curve['sy'])",quote(&format!("{mode:?}")),quote(&format!("{method:?}")),if o.step{"step"}else{"plot"},if o.step{"where='post', "}else{""},number(o.line_width),number(o.marker_size)));
        }
        lines.push("for ax in axes:".into());
        lines.push(format!("{indent}ax.grid({})", boolean(o.grid)));
        if o.log_x {
            lines.push(format!("{indent}ax.set_xscale('log', nonpositive='clip')"));
        }
        if o.log_y {
            lines.push(format!("{indent}ax.set_yscale('log', nonpositive='clip')"));
        }
        if !o.x_label.is_empty() {
            lines.push(format!("{indent}ax.set_xlabel({})", quote(&o.x_label)));
        }
        if !o.y_label.is_empty() {
            lines.push(format!("{indent}ax.set_ylabel({})", quote(&o.y_label)));
        }
        if let Some((min, max)) = o.x_limits {
            lines.push(format!(
                "{indent}ax.set_xlim({}, {})",
                number(min),
                number(max)
            ));
        }
        if let Some((min, max)) = o.y_limits {
            lines.push(format!(
                "{indent}ax.set_ylim({}, {})",
                number(min),
                number(max)
            ));
        }
        if o.flip_x {
            lines.push(format!("{indent}ax.invert_xaxis()"));
        }
        if o.flip_y {
            lines.push(format!("{indent}ax.invert_yaxis()"));
        }
        if o.legend {
            lines.push(format!(
                "{indent}if ax.lines:\n{indent}{indent}ax.legend(loc={})",
                quote(&o.legend_position.to_lowercase())
            ));
        }
        if !o.title.is_empty() {
            lines.push(format!("fig.suptitle({})", quote(&o.title)));
        }
        lines.push("fig.tight_layout()".into());
        if o.show {
            lines.push("plt.show()".into());
        }
        Ok(lines.join("\n") + "\n")
    }
}
