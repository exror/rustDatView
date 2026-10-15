use super::*;
use dv_core::{DataFrame, FileObject};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new(extension: &str, bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "datview-output-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            extension
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
#[test]
fn reads_all_fast_binary_ids() {
    for id in 1i16..=4 {
        let mut b = Vec::new();
        b.extend(id.to_le_bytes());
        if id == 4 {
            b.extend(16i16.to_le_bytes());
        }
        b.extend(1i32.to_le_bytes());
        b.extend(2i32.to_le_bytes());
        b.extend(if id == 1 { 10.0f64 } else { 1.0f64 }.to_le_bytes());
        b.extend(if id == 1 { 0.0f64 } else { 0.5f64 }.to_le_bytes());
        if id != 3 {
            b.extend(2.0f32.to_le_bytes());
            b.extend(1.0f32.to_le_bytes());
        }
        b.extend(4i32.to_le_bytes());
        b.extend(b"test");
        let width = if id == 4 { 16 } else { 10 };
        for s in ["Time", "WindSpeed", "(sec)", "(m/s)"] {
            b.extend(s.as_bytes());
            b.extend(vec![b' '; width - s.len()]);
        }
        if id == 1 {
            b.extend(10i32.to_le_bytes());
            b.extend(15i32.to_le_bytes());
        }
        if id == 3 {
            b.extend(2.0f64.to_le_bytes());
            b.extend(4.0f64.to_le_bytes());
        } else {
            b.extend(5i16.to_le_bytes());
            b.extend(9i16.to_le_bytes());
        }
        let fixture = Fixture::new("outb", &b);
        let f = load_binary_output(&fixture.0).unwrap();
        assert_eq!(f.data.names(), ["Time_[s]", "WindSpeed_[m/s]"]);
        assert_eq!(f.data.column(0).floats().unwrap(), [1.0, 1.5]);
        assert_eq!(f.data.column(1).floats().unwrap(), [2.0, 4.0]);
        assert_eq!(f.file_id, Some(id));
    }
}
#[test]
fn binary_export_preserves_long_names_and_constant_channels() {
    let df = DataFrame::from_floats(
        ["Time_[s]", "VeryLongChannelName_[m/s]", "Constant_[N]"],
        vec![vec![0.0, 0.5, 1.0], vec![-4.0, 2.0, 8.0], vec![3.0; 3]],
    )
    .unwrap();
    let fixture = Fixture::new("outb", b"x");
    write_outb(&df, &fixture.0).unwrap();
    let f = FastOutputFile::read(&fixture.0).unwrap();
    assert_eq!(f.data.names(), df.names());
    for c in 0..df.ncols() {
        for (&a, &b) in f
            .data
            .column(c)
            .floats()
            .unwrap()
            .iter()
            .zip(df.column(c).floats().unwrap())
        {
            assert!((a - b).abs() < 0.001);
        }
    }
}
#[test]
fn ascii_output_supports_utf16_and_hydro_footer() {
    let text = "OpenFAST output\n\nTime Alpha\n(sec) (deg)\n0 2\n1 3\nThis is the end\n";
    let mut bytes = vec![0xff, 0xfe];
    for v in text.encode_utf16() {
        bytes.extend(v.to_le_bytes());
    }
    let fixture = Fixture::new("out", &bytes);
    let f = load_ascii_output(&fixture.0).unwrap();
    assert_eq!(f.data.names(), ["Time_[s]", "Alpha_[deg]"]);
    assert_eq!(f.data.nrows(), 2);
}
#[test]
fn linearization_operating_points_and_state_removal() {
    let text="Simulation time: 1\nAzimuth: 0\nRotor Speed: 2\nWind Speed: 10\nNumber of continuous states: 2\nNumber of discrete states: 0\nNumber of constraint states: 0\nNumber of inputs: 1\nNumber of outputs: 1\nJacobians included in this file? True\nOrder of continuous states:\nIndex Operating Point Rotating Frame Derivative Order Description\n---\n1 0 F 2 ED First time derivative of horizontal surge translation DOF, m\n2 1 F 2 ED horizontal sway translation DOF, m\nOrder of inputs:\nIndex Operating Point Rotating Frame Description\n---\n1 3 F Generator torque (N-m)\nA:\n0 1\n-2 -3\nB:\n1\n2\nC:\n3 4\nD:\n5\n";
    let fixture = Fixture::new("lin", text.as_bytes());
    let mut f = FastLinearizationFile::read(&fixture.0).unwrap();
    assert_eq!(f.matrices["A"].values, [0.0, 1.0, -2.0, -3.0]);
    assert_eq!(
        short_description(&f.operating_points["x"].description[0]),
        "d_Surge_[m]"
    );
    f.remove_states(&[1]).unwrap();
    assert_eq!(f.matrices["A"].values, [0.0]);
    assert_eq!(f.matrices["B"].values, [1.0]);
    assert_eq!(f.matrices["C"].values, [3.0]);
}
#[test]
fn turbsim_point_time_series_roundtrip() {
    let text="TurbSim user time series\n3 nComp\n2 nPoints\n1 RefPtID\nPointyi Pointzi\n(m) (m)\n0 10\n2 20\n--------Time Series\nTime P1u P2u P1v P2v P1w P2w\n(s) (m/s) (m/s) (m/s) (m/s) (m/s) (m/s)\n0 1 2 3 4 5 6\n1 7 8 9 10 11 12\n";
    let fixture = Fixture::new("txt", text.as_bytes());
    let f = TurbSimTsFile::read(&fixture.0).unwrap();
    assert_eq!(f.data.ncols(), 7);
    assert_eq!(f.to_dataframes().unwrap()[0].name, "Points");
    assert_eq!(f.data.names()[6], "Point2w_[m/s]");
    f.write(&fixture.0).unwrap();
    let read = TurbSimTsFile::read(&fixture.0).unwrap();
    assert_eq!(read.data, f.data);
    assert_eq!(read.points, f.points);
}
#[test]
fn turbsim_grid_order_tower_and_periodic_mirror() {
    let mut b = Vec::new();
    b.extend(7i16.to_le_bytes());
    for n in [2i32, 2, 1, 3] {
        b.extend(n.to_le_bytes());
    }
    for v in [5.0f32, 4.0, 0.5, 10.0, 20.0, 10.0] {
        b.extend(v.to_le_bytes());
    }
    for _ in 0..3 {
        b.extend(2.0f32.to_le_bytes());
        b.extend(1.0f32.to_le_bytes());
    }
    b.extend(0i32.to_le_bytes());
    for t in 0..3 {
        for z in 0..2 {
            for y in 0..2 {
                for k in 0..3 {
                    b.extend(((t * 20 + z * 6 + y * 3 + k) * 2 + 1i16).to_le_bytes());
                }
            }
        }
        for k in 0..3 {
            b.extend((201i16 + k * 2 + t * 20).to_le_bytes());
        }
    }
    let fixture = Fixture::new("bts", &b);
    let mut f = TurbSimFile::read(&fixture.0).unwrap();
    assert_eq!(f.y, [-2.0, 2.0]);
    assert_eq!(f.z, [10.0, 15.0]);
    assert_eq!(f.value(2, 1, 1, 1), 31.0);
    assert_eq!(f.tower_velocity[0], 100.0);
    assert_eq!(f.fields(2, 2, 2).unwrap().len(), 30);
    f.make_periodic().unwrap();
    assert_eq!(f.id, 8);
    assert_eq!(f.t, [0.0, 0.5, 1.0, 1.5]);
    assert_eq!(f.value(2, 3, 1, 1), 31.0);
}
#[test]
fn summary_yaml_modes_and_beamdyn_table() {
    let text = "# BeamDyn summary\nInit_Nodes_E1:\n - [0, 1, 2]\n - [3, 4, 5]\n";
    let fixture = Fixture::new("yaml", text.as_bytes());
    let f = FastSummaryFile::read(&fixture.0).unwrap();
    assert_eq!(f.to_dataframes().unwrap()[0].df.nrows(), 2);
    let text="# SubDyn summary\nNodes:\n - [1, 0, 0, 10]\nDOF2Nodes:\n - [1, 6, 1]\n - [1, 6, 2]\n - [1, 6, 3]\nDOF___B: [1]\nDOF___L: [2,3]\nDOF___F: []\nPhiR:\n - [0.5]\n - [0]\nPhiM:\n - [0]\n - [1]\nCB_frequencies: [2]\n";
    let fixture = Fixture::new("yaml", text.as_bytes());
    let f = FastSummaryFile::read(&fixture.0).unwrap();
    let modes = f.get_modes(Some(0.1), Some(2)).unwrap();
    assert_eq!(modes.guyan.displacement[0][0], [0.1, 0.05, 0.0]);
    assert_eq!(modes.craig_bampton.displacement[0][0], [0.0, 0.0, 0.1]);
    assert_eq!(modes.frequencies, [2.0]);
}

#[test]
fn linearization_eigenanalysis_returns_frequency_and_modes() {
    use std::collections::BTreeMap;
    let mut matrices = BTreeMap::new();
    matrices.insert(
        "A".into(),
        LinearMatrix {
            rows: 2,
            columns: 2,
            values: vec![0.0, 1.0, -4.0, -0.4],
        },
    );
    let f = FastLinearizationFile {
        filename: PathBuf::new(),
        header: Vec::new(),
        operating_points: BTreeMap::new(),
        matrices,
        ed_dofs: Vec::new(),
        azimuth: 0.0,
        rotor_speed: 0.0,
        wind_speed: 0.0,
        time: 0.0,
    };
    let eva = f.eigenanalysis(true, true, true).unwrap();
    assert_eq!(eva.eigenvalues.len(), 1);
    assert!((eva.natural_frequency[0] - 1.0 / std::f64::consts::PI).abs() < 1e-10);
    assert!((eva.damping_ratio[0] - 0.1).abs() < 1e-10);
    for r in 0..2 {
        let a = &f.matrices["A"];
        let q = &eva.modes;
        let lhs = q[(0, 0)] * a.values[r * 2] + q[(1, 0)] * a.values[r * 2 + 1];
        assert!((lhs - q[(r, 0)] * eva.eigenvalues[0]).norm() < 1e-10);
    }
}

#[test]
fn output_time_radial_fields_use_row_major_values_and_radial_units() {
    let data = DataFrame::from_floats(
        ["Time_[s]", "AB1N001Alpha_[deg]", "AB1N002Alpha_[deg]"],
        vec![vec![1.0, 2.0], vec![3.0, 4.0], vec![5.0, 6.0]],
    )
    .unwrap();
    let fixture = Fixture::new("out", b"x");
    let f = FastOutputFile {
        filename: fixture.0.clone(),
        data,
        description: String::new(),
        binary: false,
        file_id: None,
    };
    let fields = f.fields(5.0, 3, None, true, false).unwrap();
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name, "(t,r)_B1Alpha_[deg]");
    assert_eq!(fields[0].x, [1.0, 2.0]);
    assert_eq!(fields[0].y, [0.0, 1.0]);
    assert_eq!(fields[0].m, [3.0, 5.0, 4.0, 6.0]);
    assert_eq!(fields[0].sy, "r [-]");
    let physical = f.fields(5.0, 3, Some(&[10.0, 20.0]), true, false).unwrap();
    assert_eq!(physical[0].y, [10.0, 20.0]);
    assert_eq!(physical[0].sy, "r [m]");
}
