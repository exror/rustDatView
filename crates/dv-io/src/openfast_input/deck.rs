use super::input::{broken, reference_path, unused};
use super::{FastInputFile, InputValue, RoscoDisconFile};
use dv_core::{FileObject, IoError, NamedFrame};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct FastInputDeck {
    pub filename: PathBuf,
    pub files: BTreeMap<String, FastInputFile>,
    pub version: String,
    pub missing_files: Vec<PathBuf>,
    pub discon: Option<RoscoDisconFile>,
}
fn canonical(s: &str) -> &str {
    match s.to_ascii_lowercase().as_str() {
        "fst" | "fast" => "FAST",
        "ed" => "ElastoDyn",
        "edtwr" => "ElastoDynTower",
        "edbld" => "ElastoDynBlade",
        "ad" => "AeroDyn15",
        "adbld" => "AeroDynBlade",
        "bd" => "BeamDyn",
        "bdbld" => "BeamDynBlade",
        "sd" => "SubDyn",
        "iw" => "InflowWind",
        "srvd" | "svd" => "ServoDyn",
        "hd" => "HydroDyn",
        "md" => "MoorDyn",
        _ => s,
    }
}
impl FastInputDeck {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        Self::read_selected(path, &["all"])
    }
    pub fn read_selected(path: &Path, selected: &[&str]) -> Result<Self, IoError> {
        let root = FastInputFile::read(path)?;
        let version = if root.get("DynamicSolve").is_some() {
            "BD_driver"
        } else if root.get("NumTurbines").is_some() && root.table("WindTurbines").is_none() {
            "AD_driver"
        } else if root.table("WindTurbines").is_some() {
            "FASTFarm"
        } else if root.get("InterpOrder").is_some() {
            "OF2"
        } else {
            "F7"
        };
        let mut d = Self {
            filename: path.into(),
            files: BTreeMap::from([("FAST".into(), root)]),
            version: version.into(),
            missing_files: Vec::new(),
            discon: None,
        };
        if d.version == "F7" {
            d.load("FAST", &["AeroFile"], "AeroDyn14", selected, None)?;
            d.load("FAST", &["TwrFile"], "ElastoDynTower", selected, None)?;
            d.load(
                "FAST",
                &["BldFile(1)", "BldFile1"],
                "ElastoDynBlade",
                selected,
                None,
            )?;
        } else if d.version == "FASTFarm" {
            if let Some(t) = d.files["FAST"].table("WindTurbines") {
                let paths: Vec<_> = t
                    .rows
                    .iter()
                    .enumerate()
                    .filter_map(|(i, r)| {
                        r.get(3)
                            .and_then(InputValue::as_str)
                            .map(|s| (format!("Turbine{}", i + 1), reference_path(path, s)))
                    })
                    .collect();
                for (name, p) in paths {
                    if selected
                        .iter()
                        .any(|s| *s == "all" || s.eq_ignore_ascii_case("FASTFarm"))
                    {
                        let child = Self::read_selected(&p, selected)?;
                        d.missing_files.extend(child.missing_files);
                        for (key, file) in child.files {
                            d.files.insert(format!("{name}/{key}"), file);
                        }
                    }
                }
            }
        } else if d.version == "BD_driver" {
            d.load("FAST", &["InputFile"], "BeamDyn", selected, None)?;
        } else {
            d.load("FAST", &["EDFile"], "ElastoDyn", selected, None)?;
            for (key, param, module, enabled) in [
                ("InflowFile", "CompInflow", "InflowWind", -1),
                ("AeroFile", "CompAero", "AeroDyn15", -1),
                ("ServoFile", "CompServo", "ServoDyn", -1),
                ("HydroFile", "CompHydro", "HydroDyn", 1),
                ("SubFile", "CompSub", "SubDyn", 1),
                ("MooringFile", "CompMooring", "MoorDyn", -1),
                ("BDBldFile(1)", "CompElast", "BeamDyn", 2),
            ] {
                let value = d.files["FAST"].integer(param).unwrap_or(0);
                if (enabled == -1 && value > 0) || value == enabled {
                    let module = if module == "AeroDyn15" && value == 1 {
                        "AeroDyn14"
                    } else if module == "MoorDyn" && value == 1 {
                        "MAP"
                    } else {
                        module
                    };
                    d.load("FAST", &[key], module, selected, None)?;
                }
            }
        }
        d.load("ElastoDyn", &["TwrFile"], "ElastoDynTower", selected, None)?;
        d.load(
            "ElastoDyn",
            &["BldFile(1)", "BldFile1"],
            "ElastoDynBlade",
            selected,
            None,
        )?;
        d.load("BeamDyn", &["BldFile"], "BeamDynBlade", selected, None)?;
        for key in ["AeroDyn15", "AeroDyn14"] {
            d.load(key, &["ADBlFile(1)"], "AeroDynBlade", selected, None)?;
            if let Some(ad) = d.files.get(key) {
                let refs: Vec<_> = match ad.get("AFNames").or_else(|| ad.get("FoilNm")) {
                    Some(InputValue::List(v)) => v
                        .iter()
                        .filter_map(InputValue::as_str)
                        .map(|s| reference_path(&ad.filename, s))
                        .collect(),
                    _ => Vec::new(),
                };
                for (i, p) in refs.iter().enumerate() {
                    let name = format!("AeroDynPolar{}", i + 1);
                    d.load_path(&name, p, selected, Some("AF"))?;
                    d.load(
                        &name,
                        &["NumCoords"],
                        &format!("AirfoilCoordinates{}", i + 1),
                        selected,
                        Some("AC"),
                    )?;
                }
            }
        }
        if let Some(s) = d.files.get("ServoDyn") {
            if let Some(p) = s.resolve_reference("DLL_InFile") {
                if wants(selected, "ServoDyn", None) && !unused(p.to_str().unwrap_or("")) {
                    match RoscoDisconFile::read(&p) {
                        Ok(f) => d.discon = Some(f),
                        Err(IoError::NotFound(_)) => d.missing_files.push(p),
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok(d)
    }
    fn load(
        &mut self,
        parent: &str,
        keys: &[&str],
        name: &str,
        selected: &[&str],
        short: Option<&str>,
    ) -> Result<(), IoError> {
        if !wants(selected, name, short) {
            return Ok(());
        }
        let p = self
            .files
            .get(parent)
            .and_then(|f| keys.iter().find_map(|k| f.resolve_reference(k)));
        if let Some(p) = p {
            self.load_path(name, &p, selected, short)?;
        }
        Ok(())
    }
    fn load_path(
        &mut self,
        name: &str,
        path: &Path,
        selected: &[&str],
        short: Option<&str>,
    ) -> Result<(), IoError> {
        if !wants(selected, name, short) {
            return Ok(());
        }
        match FastInputFile::read(path) {
            Ok(f) => {
                self.files.insert(name.into(), f);
            }
            Err(IoError::NotFound(_)) => self.missing_files.push(path.into()),
            Err(e) => return Err(e),
        }
        Ok(())
    }
    pub fn file(&self, key: &str) -> Option<&FastInputFile> {
        let key = canonical(key);
        self.files.get(key).or_else(|| {
            if key == "ElastoDyn" && self.version == "F7" {
                self.files.get("FAST")
            } else if key == "AeroDyn15" {
                self.files.get("AeroDyn14")
            } else if key == "FASTFarm" && self.version == "FASTFarm" {
                self.files.get("FAST")
            } else {
                None
            }
        })
    }
    pub fn path(&self, key: &str) -> Option<&Path> {
        Some(&self.file(key)?.filename)
    }
    pub fn resolve(&self, module: &str, key: &str) -> Option<PathBuf> {
        self.file(module)?.resolve_reference(key)
    }
    /// Write the complete loaded deck, renaming files and updating every loaded reference.
    pub fn write(&self, filename: &Path, prefix: &str, suffix: &str) -> Result<(), IoError> {
        if !self.missing_files.is_empty() {
            return Err(broken("cannot relocate an incomplete input deck"));
        }
        let dir = filename.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let mut paths = BTreeMap::new();
        for (key, file) in &self.files {
            let target = if key == "FAST" {
                filename.to_path_buf()
            } else {
                let ext = file
                    .filename
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("dat");
                dir.join(format!("{prefix}{}{suffix}.{ext}", key.replace('/', "_")))
            };
            paths.insert(file.filename.clone(), target);
        }
        if let Some(discon) = &self.discon {
            paths.insert(
                discon.filename.clone(),
                dir.join(format!("{prefix}DISCON{suffix}.in")),
            );
        }
        for file in self.files.values() {
            let mut f = file.clone();
            for e in &mut f.entries {
                if e.is_comment {
                    continue;
                }
                rewrite_value(&mut e.value, &file.filename, &paths);
            }
            let target = paths
                .get(&file.filename)
                .ok_or_else(|| broken("missing deck destination"))?;
            f.write(target)?;
        }
        if let Some(discon) = &self.discon {
            let mut f = discon.clone();
            for e in &mut f.entries {
                rewrite_value(&mut e.value, &discon.filename, &paths);
            }
            f.write(&paths[&discon.filename])?;
        }
        Ok(())
    }
}
fn wants(selected: &[&str], name: &str, short: Option<&str>) -> bool {
    selected.iter().any(|s| {
        *s == "all"
            || canonical(s).eq_ignore_ascii_case(name)
            || short.is_some_and(|short| s.eq_ignore_ascii_case(short))
            || (name == "AeroDyn14" && canonical(s) == "AeroDyn15")
            || (name == "MAP" && canonical(s) == "MoorDyn")
    })
}
fn rewrite_value(v: &mut InputValue, parent: &Path, paths: &BTreeMap<PathBuf, PathBuf>) {
    match v {
        InputValue::Text(s) => {
            let p = reference_path(parent, s.trim_matches(['"', '\'']).trim_start_matches('@'));
            if let Some(dest) = paths.get(&p) {
                let marker = if s.trim_matches(['"', '\'']).starts_with('@') {
                    "@"
                } else {
                    ""
                };
                *s = format!(
                    "\"{marker}{}\"",
                    dest.file_name().unwrap().to_string_lossy()
                );
            }
        }
        InputValue::List(v) => {
            for x in v {
                rewrite_value(x, parent, paths)
            }
        }
        InputValue::Table(t) => {
            for r in &mut t.rows {
                for x in r {
                    rewrite_value(x, parent, paths)
                }
            }
        }
        _ => {}
    }
}
impl FileObject for FastInputDeck {
    fn format_name(&self) -> &str {
        "FAST input deck"
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        let mut result = Vec::new();
        for (key, file) in &self.files {
            for mut f in file.to_dataframes()? {
                f.name = if f.name.is_empty() {
                    key.clone()
                } else {
                    format!("{key}/{}", f.name)
                };
                result.push(f);
            }
        }
        Ok(result)
    }
}
