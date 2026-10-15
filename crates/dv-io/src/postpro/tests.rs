use super::*;

fn frame(names: &[&str], columns: Vec<Vec<f64>>) -> DataFrame {
    DataFrame::from_floats(names.iter().copied(), columns).unwrap()
}
#[test]
fn constant_window_includes_both_endpoints_and_skips_nan() {
    let df = frame(
        &["Time_[s]", "Load"],
        vec![
            vec![0., 1., 2., 3., f64::NAN],
            vec![100., 2., f64::NAN, 6., 999.],
        ],
    );
    let avg = average_df(&df, &AverageOptions::constant_window(2.0)).unwrap();
    assert_eq!(avg.get("Time_[s]").unwrap().floats().unwrap(), &[2.]);
    assert_eq!(avg.get("Load").unwrap().floats().unwrap(), &[4.]);
    assert!(average_df(&df, &AverageOptions::constant_window(-1.)).is_err());
}
#[test]
fn periods_use_last_up_crossing_sample_not_interpolated_time() {
    let df = frame(
        &["Time_[s]", "Azimuth_[deg]"],
        vec![
            (0..12).map(|i| i as f64).collect(),
            vec![
                0., 90., 180., 270., 0., 90., 180., 270., 0., 90., 180., 270.,
            ],
        ],
    );
    assert_eq!(
        find_period_time_range(&df, Some(1), None, None).unwrap(),
        (6., 10.)
    );
    assert_eq!(
        find_period_time_range(&df, None, None, None).unwrap(),
        (2., 10.)
    );
}
#[test]
fn rotation_speed_window_is_computed() {
    let df = frame(
        &["Time_[s]", "RotSpeed_[rpm]", "Load"],
        vec![
            (0..=6).map(|i| i as f64).collect(),
            vec![30.; 7],
            vec![0., 1., 2., 3., 4., 5., 6.],
        ],
    );
    let options = AverageOptions {
        method: AverageMethod::PeriodsOmega,
        parameter: Some(1.),
        ..Default::default()
    };
    assert_eq!(
        average_df(&df, &options)
            .unwrap()
            .get("Load")
            .unwrap()
            .floats()
            .unwrap(),
        &[5.]
    );
}
#[test]
fn azimuth_bins_are_right_closed_and_periodic() {
    let df = frame(
        &["Azimuth_[deg]", "Load"],
        vec![vec![0., 90., 180., 270., 360.], vec![99., 1., 3., 5., 7.]],
    );
    let opts = AzimuthOptions {
        bins: vec![0., 180., 360.],
        periodic: true,
        ..Default::default()
    };
    let bins = azimuthal_average_df(&df, &opts).unwrap();
    assert_eq!(bins.get("Load").unwrap().floats().unwrap(), &[2., 6., 2.]);
    assert_eq!(bins.get("Counts").unwrap().floats().unwrap(), &[2., 2., 2.]);
}
#[test]
fn sparse_channels_preserve_node_numbers_and_concat_order() {
    let df = frame(
        &[
            "Time_[s]",
            "AB1N003Cl_[-]",
            "AB1N001Cl_[-]",
            "AB1N001Cd_[-]",
        ],
        vec![vec![0., 1.], vec![30., 31.], vec![10., 11.], vec![1., 2.]],
    );
    let fields = spanwise_postpro_rows(&df, None).unwrap();
    let cl = fields.ad.iter().find(|f| f.name == "B1Cl_[-]").unwrap();
    assert_eq!(cl.nodes, 3);
    assert_eq!(cl.values[0], 10.);
    assert!(cl.values[1].is_nan());
    assert_eq!(cl.values[2], 30.);
    assert_eq!(cl.values[3], 11.);
    assert!(cl.values[4].is_nan());
    assert_eq!(cl.values[5], 31.);
    assert_eq!(cl.radii, vec![0., 0.5, 1.]);
    let cat = spanwise_concat(&df).unwrap();
    assert_eq!(
        cat.get("Time_[s]").unwrap().floats().unwrap(),
        &[0., 1., 0., 1., 0., 1.]
    );
    assert_eq!(
        cat.get("i_[-]").unwrap().floats().unwrap(),
        &[1., 1., 2., 2., 3., 3.]
    );
    let cl = cat.get("B1Cl_[-]").unwrap().floats().unwrap();
    assert_eq!(&cl[..2], &[10., 11.]);
    assert!(cl[2].is_nan());
    assert_eq!(&cl[4..], &[30., 31.]);
}
#[test]
fn legacy_elastodyn_moment_mapping_is_preserved() {
    let df = frame(
        &["Spn1MLyb1_[kN-m]", "Spn2MLxb1_[kN-m]"],
        vec![vec![4.], vec![5.]],
    );
    let channels = spanwise_columns(&df, SpanwiseModule::ElastoDyn);
    assert!(channels
        .iter()
        .any(|c| c.name == "B1MLx_[kN-m]" && c.indices == vec![1]));
    assert!(channels
        .iter()
        .any(|c| c.name == "B1MLy_[kN-m]" && c.indices == vec![2]));
}
#[test]
fn farm_extracts_multi_digit_turbines_and_downstream_stations() {
    let df = frame(
        &[
            "Time_[s]",
            "CtT12N01_[-]",
            "CtT12N03_[-]",
            "WkDfVxT12N01D10_[m/s]",
            "WkPosXT12D2_[m]",
        ],
        vec![
            vec![0., 1.],
            vec![2., 4.],
            vec![6., 8.],
            vec![10., 12.],
            vec![14., 16.],
        ],
    );
    let positions = FarmPositions {
        radial: vec![2., 4., 6.],
        downstream: vec![100., 200.],
    };
    let out =
        spanwise_postpro_ff(&df, &AverageOptions::constant_window(1.), Some(&positions)).unwrap();
    assert_eq!(
        out.radial.get("r_[m]").unwrap().floats().unwrap(),
        &[2., 4., 6.]
    );
    assert_eq!(
        out.radial.get("CtT12_[-]").unwrap().floats().unwrap()[2],
        7.
    );
    assert_eq!(
        out.radial.get("i/n_[-]").unwrap().floats().unwrap(),
        &[0., 1. / 3., 2. / 3.]
    );
    assert_eq!(
        out.diameter.get("WkPosXT12_[m]").unwrap().floats().unwrap()[1],
        15.
    );
    assert_eq!(out.radial_time.ncols(), 4);
}
#[test]
fn subdyn_rotations_convert_units_and_omit_unused_points() {
    let geometry = SubDynGeometry {
        output_nodes: vec![
            SubDynPoint {
                label: "M1N1".into(),
                coordinates: [1., 2., 3.],
            },
            SubDynPoint {
                label: "M1N2".into(),
                coordinates: [4., 5., 6.],
            },
        ],
        member_joints: vec![SubDynPoint {
            label: "M1J1".into(),
            coordinates: [0., 0., 0.],
        }],
        ..Default::default()
    };
    let avg = frame(
        &["M1N2RDxe_[rad]", "M1J1FKxe_[N]"],
        vec![vec![std::f64::consts::PI], vec![12.]],
    );
    let (members, joints) = geometry.member_postpro(&avg).unwrap();
    assert_eq!(members.nrows(), 1);
    assert_eq!(
        members.get("RDxe_[deg]").unwrap().floats().unwrap(),
        &[180.]
    );
    assert_eq!(members.get("x_[m]").unwrap().floats().unwrap(), &[4.]);
    assert_eq!(joints.get("FKxe_[N]").unwrap().floats().unwrap(), &[12.]);
}
#[test]
fn radial_geometry_enforces_lengths_and_normalizes_radius() {
    let mut df = frame(&["Load"], vec![vec![2., 3.]]);
    let pos = StationPositions {
        radii: vec![10., 20.],
        node_indices: Some(vec![3, 5]),
    };
    insert_spanwise_columns(&mut df, Some(&pos), Some(40.), "r").unwrap();
    assert_eq!(df.get("r/R_[-]").unwrap().floats().unwrap(), &[0.25, 0.5]);
    assert_eq!(df.get("Node_[#]").unwrap().floats().unwrap(), &[3., 5.]);
    let short = StationPositions {
        radii: vec![1.],
        node_indices: None,
    };
    assert!(insert_spanwise_columns(&mut df, Some(&short), None, "r").is_err());
}

fn input(
    values: Vec<(&str, crate::openfast_input::InputValue)>,
) -> crate::openfast_input::FastInputFile {
    use crate::openfast_input::{FastInputFile, InputEntry};
    FastInputFile {
        filename: std::path::PathBuf::new(),
        module: None,
        has_nodal: false,
        entries: values
            .into_iter()
            .map(|(label, value)| InputEntry {
                label: label.into(),
                value,
                description: String::new(),
                is_comment: false,
                dimension: None,
                table_type: 0,
            })
            .collect(),
    }
}
fn input_table(names: &[&str], rows: &[Vec<f64>]) -> crate::openfast_input::InputValue {
    use crate::openfast_input::{InputTable, InputValue};
    InputValue::Table(InputTable {
        columns: names.iter().map(|s| s.to_string()).collect(),
        units: vec![],
        rows: rows
            .iter()
            .map(|r| r.iter().map(|v| InputValue::Number(*v)).collect())
            .collect(),
    })
}
#[test]
fn elastodyn_stations_and_selected_gages_have_half_cell_offset() {
    use crate::openfast_input::InputValue as V;
    let ed = input(vec![
        ("BldNodes", V::Integer(4)),
        ("TipRad", V::Number(42.)),
        ("HubRad", V::Number(2.)),
        ("NBlGages", V::Integer(2)),
        ("BldGagNd", V::List(vec![V::Integer(1), V::Integer(4)])),
        ("TwrNodes", V::Integer(2)),
        ("TowerHt", V::Number(105.)),
        ("TowerBsHt", V::Number(5.)),
        ("NTwGages", V::Integer(1)),
        ("TwrGagNd", V::Integer(2)),
    ]);
    assert_eq!(ed_blade_stations(&ed).unwrap().1, vec![7., 17., 27., 37.]);
    assert_eq!(ed_blade_gages(&ed).unwrap().radii, vec![7., 37.]);
    assert_eq!(ed_tower_stations(&ed, false).unwrap().1, vec![25., 75.]);
    assert_eq!(ed_tower_gages(&ed, true).unwrap().radii, vec![80.]);
}
#[test]
fn beamdyn_gauss_and_refined_trapezoidal_stations() {
    use crate::openfast_input::InputValue as V;
    let geom = input_table(&["x", "y", "z"], &[vec![0., 0., 0.], vec![0., 0., 10.]]);
    let gauss = input(vec![
        ("MemberGeom", geom.clone()),
        ("quadrature", V::Integer(1)),
        ("order_elem", V::Integer(1)),
    ]);
    let blade = input(vec![(
        "BeamProperties",
        input_table(&["Span"], &[vec![0.], vec![0.25], vec![1.]]),
    )]);
    let r = bd_blade_stations(&gauss, &blade).unwrap();
    assert!((r[0] - 5. * (1. - 1. / 3_f64.sqrt())).abs() < 1e-12);
    assert!((r[1] - 5. * (1. + 1. / 3_f64.sqrt())).abs() < 1e-12);
    let trap = input(vec![
        ("MemberGeom", geom),
        ("quadrature", V::Integer(2)),
        ("refine", V::Integer(2)),
    ]);
    assert_eq!(
        bd_blade_stations(&trap, &blade).unwrap(),
        vec![0., 1.25, 2.5, 6.25, 10.]
    );
}
#[test]
fn subdyn_selected_member_ids_are_independent_of_output_ordinals() {
    use crate::openfast_input::InputValue as V;
    let sd = input(vec![
        ("NDiv", V::Integer(2)),
        (
            "Joints",
            input_table(
                &["ID", "x", "y", "z"],
                &[
                    vec![10., 0., 0., 0.],
                    vec![20., 0., 0., 10.],
                    vec![30., 0., 0., 20.],
                ],
            ),
        ),
        (
            "Members",
            input_table(
                &["ID", "J1", "J2", "P1", "P2", "Type"],
                &[
                    vec![7., 10., 20., 1., 1., 1.],
                    vec![9., 20., 30., 1., 1., 3.],
                ],
            ),
        ),
        (
            "MemberOuts",
            input_table(&[], &[vec![9., 2., 1., 2.], vec![7., 1., 2.]]),
        ),
    ]);
    let geometry = SubDynGeometry::from_input(&sd).unwrap();
    assert_eq!(geometry.member_nodes.len(), 5);
    assert_eq!(geometry.output_nodes[0].label, "M1N1");
    assert_eq!(geometry.output_nodes[1].coordinates, [0., 0., 20.]);
    assert_eq!(geometry.output_nodes[2].label, "M2N1");
    assert_eq!(geometry.output_nodes[2].coordinates, [0., 0., 5.]);
}
#[test]
fn ad14_print_flags_select_physical_stations() {
    use crate::openfast_input::{InputTable, InputValue as V};
    let table = InputTable {
        columns: vec![],
        units: vec![],
        rows: vec![
            vec![
                V::Number(1.),
                V::Number(0.),
                V::Number(0.),
                V::Number(0.),
                V::Number(0.),
                V::Bool(false),
            ],
            vec![
                V::Number(2.),
                V::Number(0.),
                V::Number(0.),
                V::Number(0.),
                V::Number(0.),
                V::Bool(true),
            ],
        ],
    };
    let geometry = ad14_blade_gages(&input(vec![("BldAeroNodes", V::Table(table))])).unwrap();
    assert_eq!(geometry.radii, vec![2.]);
    assert_eq!(geometry.node_indices, Some(vec![2]));
}
