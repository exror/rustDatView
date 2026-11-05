use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::NaiveDate;
use dv_core::{Column, ColumnData, DataFrame, FileObject};
use dv_io::generic::csv::{self, CsvOptions};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct TempFile(PathBuf);

impl TempFile {
    fn new(extension: &str, bytes: &[u8]) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "datview-telemetry-{}-{nonce}-{sequence}.{extension}",
            std::process::id()
        ));
        std::fs::write(&path, bytes).expect("write temporary telemetry fixture");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn open(fixture: &TempFile) -> Arc<dyn FileObject> {
    dv_io::read_file(fixture.path(), None).expect("detect and read telemetry fixture")
}

fn frame(file: &dyn FileObject) -> DataFrame {
    let mut frames = file
        .to_dataframes()
        .expect("convert telemetry to dataframes");
    assert_eq!(frames.len(), 1, "expected one table: {}", file.info());
    frames.remove(0).df
}

fn column<'a>(df: &'a DataFrame, name: &str) -> &'a Column {
    df.get(name)
        .unwrap_or_else(|| panic!("missing column {name:?}: {:?}", df.names()))
}

fn floats(c: &Column) -> &[f64] {
    match &c.data {
        ColumnData::Float(values) => values,
        other => panic!("{} should be numeric, got {other:?}", c.name),
    }
}

fn strings(c: &Column) -> &[String] {
    match &c.data {
        ColumnData::Str(values) => values,
        other => panic!("{} should be text, got {other:?}", c.name),
    }
}

fn datetimes(df: &DataFrame) -> &[f64] {
    let mut matches = df.columns().iter().filter_map(|c| match &c.data {
        ColumnData::DateTime(values) => Some((c, values.as_slice())),
        _ => None,
    });
    let (c, values) = matches.next().expect("expected a datetime column");
    assert!(
        c.name.to_ascii_lowercase().contains("time")
            || c.name.to_ascii_lowercase().contains("date")
    );
    assert!(
        matches.next().is_none(),
        "expected one datetime axis: {:?}",
        df.names()
    );
    values
}

fn epoch(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32, millis: u32) -> f64 {
    let date = NaiveDate::from_ymd_opt(year, month, day).unwrap();
    let time = date
        .and_hms_milli_opt(hour, minute, second, millis)
        .unwrap()
        .and_utc();
    time.timestamp() as f64 + f64::from(time.timestamp_subsec_millis()) / 1000.0
}

fn near(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-6,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn utf8_bom_and_windows_1252_decode_before_csv_parsing() {
    let bom = TempFile::new(
        "csv",
        b"\xef\xbb\xbfTime [s],Power [W]\r\n0,3.5\r\n1,4.5\r\n",
    );
    let file = open(&bom);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 2));
    assert_eq!(df.names(), ["Time [s]", "Power [W]"]);
    assert_eq!(floats(column(&df, "Power [W]")), [3.5, 4.5]);
    assert!(file.info().to_ascii_lowercase().contains("utf-8"));
    assert!(file.info().to_ascii_lowercase().contains("bom"));

    // These bytes are printable Windows-1252 characters, not Latin-1 controls.
    let ansi = TempFile::new(
        "csv",
        b"Time [s],Cost \x80,Note\r\n0,1,\x91hot\x92\x85\x96\x99\r\n1,2,ok\r\n",
    );
    let file = open(&ansi);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    assert_eq!(floats(column(&df, "Cost €")), [1.0, 2.0]);
    assert_eq!(strings(column(&df, "Note")), ["‘hot’…–™", "ok"]);
    assert!(file.info().to_ascii_lowercase().contains("windows-1252"));
}

#[test]
fn utf16_both_byte_orders_are_decoded_before_delimiter_detection() {
    let text =
        "\"Time [s]\",\"Temperature [°C]\",\"Current [µA]\"\r\n0,32.5,1200\r\n1,33.5,1300\r\n";
    for (little_endian, label) in [(true, "utf-16le"), (false, "utf-16be")] {
        let mut bytes = if little_endian {
            vec![0xff, 0xfe]
        } else {
            vec![0xfe, 0xff]
        };
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&if little_endian {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        let fixture = TempFile::new("csv", &bytes);
        let file = open(&fixture);
        let df = frame(file.as_ref());
        assert_eq!((df.nrows(), df.ncols()), (2, 3));
        assert_eq!(floats(column(&df, "Temperature [°C]")), [32.5, 33.5]);
        assert_eq!(floats(column(&df, "Current [µA]")), [1200.0, 1300.0]);
        assert!(
            file.info().to_ascii_lowercase().contains(label),
            "{}",
            file.info()
        );
    }
}

#[test]
fn quoted_delimiters_escaped_quotes_and_newlines_are_one_record() {
    let fixture = TempFile::new(
        "csv",
        b"Time [s],Label,Value\n0,\"heat, \"\"phase\"\"\nresume\",1.25\n1,done,2.5\n",
    );
    let df = frame(open(&fixture).as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    assert_eq!(
        strings(column(&df, "Label")),
        ["heat, \"phase\"\nresume", "done"]
    );
    assert_eq!(floats(column(&df, "Value")), [1.25, 2.5]);
}

#[test]
fn semicolon_decimal_comma_and_grouped_thousands_do_not_shift_fields() {
    let fixture = TempFile::new(
        "csv",
        b"Zeit [s];Leistung [W];Strom [A]\n0,000;1.234,56;0,573\n1,500;2.345,67;0,650\n",
    );
    let df = frame(open(&fixture).as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    assert_eq!(floats(column(&df, "Zeit [s]")), [0.0, 1.5]);
    assert_eq!(floats(column(&df, "Leistung [W]")), [1234.56, 2345.67]);
    assert_eq!(floats(column(&df, "Strom [A]")), [0.573, 0.65]);
}

#[test]
fn pipe_and_whitespace_tables_keep_their_columns() {
    let pipe = TempFile::new(
        "csv",
        b"Time [s]|Power [W]|Label\n0|3.5|\"GPU|CPU\"\n1|4.5|idle\n",
    );
    let df = frame(open(&pipe).as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    assert_eq!(floats(column(&df, "Power [W]")), [3.5, 4.5]);
    assert_eq!(strings(column(&df, "Label")), ["GPU|CPU", "idle"]);

    let whitespace = TempFile::new(
        "txt",
        b"Time_Of_Day_Seconds Core CPU PkgWatt\n1679142896.7 - - 28.56\n1679142897.7 0 0 29.25\n",
    );
    let df = frame(open(&whitespace).as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 4));
    let time = datetimes(&df);
    near(time[0], epoch(2023, 3, 18, 12, 34, 56, 700));
    near(time[1] - time[0], 1.0);
    assert_eq!(floats(column(&df, "PkgWatt")), [28.56, 29.25]);
    let core = floats(column(&df, "Core"));
    assert!(core[0].is_nan());
    assert_eq!(core[1], 0.0);
}

#[test]
fn commented_metadata_is_kept_without_becoming_data() {
    let fixture = TempFile::new(
        "csv",
        b"# Synthetic monitor export\n# Tool,Example PTAT\n# Sampling interval,1 s\nTimestamp,Index,Power [W]\n2023-03-18 12:34:56,0,28.562\n2023-03-18 12:34:57,0,29.125\n",
    );
    let file = open(&fixture);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    assert_eq!(floats(column(&df, "Power [W]")), [28.562, 29.125]);
    near(datetimes(&df)[0], epoch(2023, 3, 18, 12, 34, 56, 0));
    let info = file.info();
    assert!(info.contains("Example PTAT"), "{info}");
    assert!(info.contains("Sampling interval"), "{info}");
}

#[test]
fn hardware_monitor_two_header_rows_keep_duplicate_sensors_and_ids() {
    let fixture = TempFile::new(
        "csv",
        b",/intelcpu/0/power/0,/intelcpu/0/temperature/0,/gpu-nvidia/0/fan/0\nTime,\"CPU Package\",\"CPU Package\",\"GPU Fan\"\n03/18/2023 12:34:56,28.562,32.089,600\n03/18/2023 12:34:57,29.125,33.0,610\n",
    );
    let file = open(&fixture);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 4));
    near(datetimes(&df)[0], epoch(2023, 3, 18, 12, 34, 56, 0));
    let packages: Vec<_> = df
        .columns()
        .iter()
        .filter(|c| c.name.starts_with("CPU Package"))
        .collect();
    assert_eq!(packages.len(), 2, "{:?}", df.names());
    assert_eq!(floats(packages[0]), [28.562, 29.125]);
    assert_eq!(floats(packages[1]), [32.089, 33.0]);
    assert_eq!(packages[0].name, "CPU Package [W]");
    assert_eq!(packages[1].name, "CPU Package [°C]");
    assert_eq!(
        column(&df, "GPU Fan [RPM]").floats().unwrap(),
        &[600.0, 610.0]
    );
    let identity = format!("{} {:?}", file.info(), df.names());
    assert!(identity.contains("/intelcpu/0/power/0"), "{identity}");
    assert!(identity.contains("/intelcpu/0/temperature/0"), "{identity}");
}

#[test]
fn hwinfo_merges_time_and_disambiguates_dates_over_the_whole_column() {
    let fixture = TempFile::new(
        "csv",
        b"Date,Time,CPU Thermal Throttling,GPU Fan [RPM],GPU Fan [RPM],\n03/04/2023,12:00:00.250,No,450,456,\n13/04/2023,12:00:00.500,Yes,451,457,\nDate,Time,CPU Thermal Throttling,GPU Fan [RPM],GPU Fan [RPM],\n14/04/2023,12:00:00.750,No,452,458,\nDate,Time,CPU Thermal Throttling,GPU Fan [RPM],GPU Fan [RPM],\nSensor Group: CPU [#0],CPU Enhanced,GPU [#0]\n",
    );
    let file = open(&fixture);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (3, 4), "{}", file.info());
    let time = datetimes(&df);
    near(time[0], epoch(2023, 4, 3, 12, 0, 0, 250));
    near(time[1], epoch(2023, 4, 13, 12, 0, 0, 500));
    near(time[2], epoch(2023, 4, 14, 12, 0, 0, 750));
    let fans: Vec<_> = df
        .columns()
        .iter()
        .filter(|c| c.name.starts_with("GPU Fan") && c.name.ends_with("[RPM]"))
        .collect();
    assert_eq!(
        fans.len(),
        2,
        "duplicate sensors must remain distinct: {:?}",
        df.names()
    );
    assert_eq!(floats(fans[0]), [450.0, 451.0, 452.0]);
    assert_eq!(floats(fans[1]), [456.0, 457.0, 458.0]);
    let flag = df
        .columns()
        .iter()
        .find(|c| c.name.starts_with("CPU Thermal Throttling"))
        .expect("thermal throttling flag");
    match &flag.data {
        ColumnData::Str(values) => assert_eq!(values, &["No", "Yes", "No"]),
        ColumnData::Float(values) => assert_eq!(values, &[0.0, 1.0, 0.0]),
        other => panic!("flag is not categorical or boolean: {other:?}"),
    }
    let info = file.info().to_ascii_lowercase();
    assert!(info.contains("2 repeated header"), "{info}");
    assert!(info.contains("1 sensor-group"), "{info}");
    assert!(info.contains("terminal column"), "{info}");
}

#[test]
fn explicit_dayfirst_changes_ambiguous_dates_not_numeric_values() {
    let fixture = TempFile::new(
        "csv",
        b"Date,Time,Power [W]\n03/04/2023,12:00:00,1\n04/05/2023,12:00:01,2\n",
    );
    let month_first = csv::read_with_options(fixture.path(), &CsvOptions::default()).unwrap();
    let day_first = csv::read_with_options(
        fixture.path(),
        &CsvOptions {
            dayfirst: true,
            ..CsvOptions::default()
        },
    )
    .unwrap();
    let m = frame(month_first.as_ref());
    let d = frame(day_first.as_ref());
    assert_eq!((m.nrows(), d.nrows()), (2, 2));
    near(datetimes(&m)[0], epoch(2023, 3, 4, 12, 0, 0, 0));
    near(datetimes(&d)[0], epoch(2023, 4, 3, 12, 0, 0, 0));
    near(datetimes(&d)[1], epoch(2023, 5, 4, 12, 0, 1, 0));
    assert_eq!(floats(column(&d, "Power [W]")), [1.0, 2.0]);
}

#[test]
fn nvidia_value_units_missing_cells_and_hex_identifiers() {
    let fixture = TempFile::new(
        "csv",
        b"timestamp,index,name,utilization.gpu [%],clocks.gr [MHz],power.draw [W],memory.used [MiB],fan.speed [%],address,throttle_flags\n2023/03/18 12:34:56.700,0,Synthetic GPU A,14 %,640 MHz,58.46 W,1896 MiB,18 %,0xfedcba9876543210,0x0\n2023/03/18 12:34:56.700,1,Synthetic GPU B,7 %,288 MHz,26.30 W,853 MiB,[N/A],0x0000000000000001,0x2\n",
    );
    let df = frame(open(&fixture).as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 10));
    assert_eq!(floats(column(&df, "index")), [0.0, 1.0]);
    assert_eq!(
        strings(column(&df, "name")),
        ["Synthetic GPU A", "Synthetic GPU B"]
    );
    assert_eq!(floats(column(&df, "utilization.gpu [%]")), [14.0, 7.0]);
    assert_eq!(floats(column(&df, "clocks.gr [MHz]")), [640.0, 288.0]);
    assert_eq!(floats(column(&df, "power.draw [W]")), [58.46, 26.3]);
    assert_eq!(floats(column(&df, "memory.used [MiB]")), [1896.0, 853.0]);
    let fan = floats(column(&df, "fan.speed [%]"));
    assert_eq!(fan[0], 18.0);
    assert!(fan[1].is_nan());
    assert_eq!(
        strings(column(&df, "address")),
        ["0xfedcba9876543210", "0x0000000000000001"]
    );
    assert_eq!(floats(column(&df, "throttle_flags")), [0.0, 2.0]);
    let time = datetimes(&df);
    near(time[0], epoch(2023, 3, 18, 12, 34, 56, 700));
    assert_eq!(time[0], time[1]);
}

#[test]
fn short_rows_are_missing_and_long_rows_are_not_silently_truncated() {
    let short = TempFile::new("csv", b"Time [s],Power [W],Fan [RPM]\n0,3,500\n1,4\n");
    let short_file = open(&short);
    let df = frame(short_file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    assert_eq!(floats(column(&df, "Power [W]")), [3.0, 4.0]);
    let fan = floats(column(&df, "Fan [RPM]"));
    assert_eq!(fan[0], 500.0);
    assert!(fan[1].is_nan());
    let info = short_file.info().to_ascii_lowercase();
    assert!(
        info.contains("1 short") && info.contains("padded"),
        "{info}"
    );

    let long = TempFile::new("csv", b"Time [s],Power [W]\n0,3\n1,4,99\n");
    let file = csv::read_with_options(long.path(), &CsvOptions::default()).unwrap();
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    assert_eq!(floats(column(&df, "Power [W]")), [3.0, 4.0]);
    let extra = floats(df.column(2));
    assert!(extra[0].is_nan());
    assert_eq!(extra[1], 99.0);
    let info = file.info().to_ascii_lowercase();
    assert!(
        info.contains("1 long") && info.contains("preserved"),
        "{info}"
    );
}

#[test]
fn utc_timestamps_and_numeric_elapsed_seconds_have_distinct_types() {
    let fixture = TempFile::new(
        "csv",
        b"Timestamp,Elapsed [s],Power [W]\n2023-03-18T12:34:56.700Z,0,10\n2023-03-18T14:34:57.700+02:00,1,11\n",
    );
    let df = frame(open(&fixture).as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    let time = datetimes(&df);
    near(time[0], epoch(2023, 3, 18, 12, 34, 56, 700));
    near(time[1], epoch(2023, 3, 18, 12, 34, 57, 700));
    assert_eq!(floats(column(&df, "Elapsed [s]")), [0.0, 1.0]);

    let compact = TempFile::new(
        "csv",
        b"#time,current [A]\n20230318T123456.700Z,0.5\n20230318T123457.700Z,0.6\n",
    );
    let df = frame(open(&compact).as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 2));
    near(datetimes(&df)[0], epoch(2023, 3, 18, 12, 34, 56, 700));
    assert_eq!(floats(column(&df, "current [A]")), [0.5, 0.6]);
}

#[test]
fn log_extension_dispatches_gpu_sensor_csv() {
    let fixture = TempFile::new(
        "log",
        b"Date, GPU Core Clock [MHz], GPU Temperature [C],\n2023-03-18 12:34:56, 135.0, 44.0,\n2023-03-18 12:34:57, 140.0, 45.0,\n",
    );
    let file = open(&fixture);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    assert_eq!(floats(column(&df, "GPU Core Clock [MHz]")), [135.0, 140.0]);
    assert_eq!(floats(column(&df, "GPU Temperature [C]")), [44.0, 45.0]);
    near(datetimes(&df)[0], epoch(2023, 3, 18, 12, 34, 56, 0));
}

#[test]
fn scope_preamble_and_separate_units_are_not_samples() {
    let fixture = TempFile::new(
        "csv",
        b"Model,DS1054Z,Record Length,2\nChannel,CH1,Sample Interval,1.000000e-6 s\nX,CH1\ns,V\n-5.000000e-4,1.2\n-4.990000e-4,1.3\n",
    );
    let file = open(&fixture);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 2));
    assert!(
        df.column(0).name.starts_with('X') && df.column(0).name.contains('s'),
        "{:?}",
        df.names()
    );
    assert!(
        df.column(1).name.starts_with("CH1") && df.column(1).name.contains('V'),
        "{:?}",
        df.names()
    );
    assert_eq!(floats(df.column(0)), [-0.0005, -0.000499]);
    assert_eq!(floats(df.column(1)), [1.2, 1.3]);
    let info = file.info();
    assert!(info.contains("DS1054Z"), "{info}");
    assert!(info.contains("Sample Interval"), "{info}");
}

#[test]
fn tek_scope_reads_waveform_in_later_columns() {
    let fixture = TempFile::new(
        "csv",
        b"Record Length,2,,TIME,CH1\nSample Interval,1.000000e-6,,s,V\nTrigger Point,1,,,\nFirmware Version,Synthetic,,,\n,,,-5.000000e-4,1.2\n,,,-4.990000e-4,1.3\n",
    );
    let file = open(&fixture);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 2), "{}", file.info());
    assert!(
        df.column(0).name.starts_with("TIME") && df.column(0).name.contains('s'),
        "{:?}",
        df.names()
    );
    assert!(
        df.column(1).name.starts_with("CH1") && df.column(1).name.contains('V'),
        "{:?}",
        df.names()
    );
    assert_eq!(floats(df.column(0)), [-0.0005, -0.000499]);
    assert_eq!(floats(df.column(1)), [1.2, 1.3]);
    let info = file.info();
    assert!(info.contains("Firmware Version"), "{info}");
    assert!(info.contains("Sample Interval"), "{info}");
}

#[test]
fn rigol_sequence_preserves_index_and_timebase_metadata() {
    let fixture = TempFile::new(
        "csv",
        b"X,CH1,Start,Increment,\nSequence,Volt,-5.000000e-4,1.000000e-6\n0,1.2\n1,1.3\n",
    );
    let file = open(&fixture);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    assert_eq!(df.column(0).name, "Time [s]");
    let time = floats(df.column(0));
    near(time[0], -0.0005);
    near(time[1], -0.000499);
    assert!(df.column(1).name.starts_with('X'), "{:?}", df.names());
    assert_eq!(floats(df.column(1)), [0.0, 1.0]);
    assert!(
        df.column(2).name.starts_with("CH1") && df.column(2).name.contains('V'),
        "{:?}",
        df.names()
    );
    assert_eq!(floats(df.column(2)), [1.2, 1.3]);
    let info = file.info();
    assert!(
        info.contains("Start") && info.contains("Increment"),
        "{info}"
    );
    assert!(
        (info.contains("-5.000000e-4") || info.contains("-5e-4") || info.contains("-0.0005"))
            && (info.contains("1.000000e-6") || info.contains("1e-6") || info.contains("0.000001")),
        "{info}"
    );
}

#[test]
fn typed_afterburner_hml_uses_metadata_and_sample_records() {
    let fixture = TempFile::new(
        "hml",
        b"00, 30-01-2025 11:53:47, Hardware monitoring log v1.6\n01, 30-01-2025 11:53:47, NVIDIA GeForce RTX 3070\n02, 30-01-2025 11:53:47, GPU temperature, GPU usage\n03, 30-01-2025 11:53:47, 0, \xb0C, 0.000, 100.000, 10\n03, 30-01-2025 11:53:47, 1, %, 0.000, 100.000, 10\n80, 30-01-2025 11:53:48, 59.000, 44.000\n80, 30-01-2025 11:53:49, 60.000, 45.000\n",
    );
    let file = open(&fixture);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 3));
    near(datetimes(&df)[0], epoch(2025, 1, 30, 11, 53, 48, 0));
    near(datetimes(&df)[1], epoch(2025, 1, 30, 11, 53, 49, 0));
    assert_eq!(floats(column(&df, "GPU temperature [°C]")), [59.0, 60.0]);
    assert_eq!(floats(column(&df, "GPU usage [%]")), [44.0, 45.0]);
    let info = file.info();
    assert!(info.contains("1.6"), "{info}");
    assert!(info.contains("NVIDIA GeForce RTX 3070"), "{info}");
    assert!(info.to_ascii_lowercase().contains("windows-1252"), "{info}");
}

#[test]
fn value_units_boolean_flags_and_named_missing_terminal_column() {
    let fixture = TempFile::new(
        "csv",
        b"Elapsed [s],Power,Enabled,Fan [RPM]\n0,45.20 W,On,\n1,46.75 W,Off,\n",
    );
    let file = open(&fixture);
    let df = frame(file.as_ref());
    assert_eq!((df.nrows(), df.ncols()), (2, 4));
    assert_eq!(floats(column(&df, "Elapsed [s]")), [0.0, 1.0]);
    assert_eq!(floats(column(&df, "Power [W]")), [45.2, 46.75]);
    assert_eq!(floats(column(&df, "Enabled [Yes/No]")), [1.0, 0.0]);
    assert!(floats(column(&df, "Fan [RPM]")).iter().all(|v| v.is_nan()));
    assert!(file.info().contains("unit suffixes"));
    assert!(file.info().contains("boolean"));
}

#[test]
fn schema_units_are_inferred_only_for_identified_logs() {
    let ptat = TempFile::new(
        "csv",
        b"Timestamp,Device,CFreq,UFreq,Temp,Power,Volt,TStat\n03/18/23 12:34:56,CPU0,1000,800,32,28,0.8,0x0\n03/18/23 12:34:57,CPU0,1100,900,33,29,0.9,0x1\n",
    );
    let df = frame(open(&ptat).as_ref());
    assert_eq!(floats(column(&df, "CFreq [MHz]")), [1000.0, 1100.0]);
    assert_eq!(floats(column(&df, "Temp [°C]")), [32.0, 33.0]);
    assert_eq!(floats(column(&df, "Power [W]")), [28.0, 29.0]);
    let joulescope = TempFile::new(
        "csv",
        b"#time,current,voltage,power,charge,energy\n0,0.1,3,0.3,0,0\n0.01,0.2,3,0.6,0.002,0.006\n",
    );
    let df = frame(open(&joulescope).as_ref());
    assert_eq!(floats(column(&df, "#time [s]")), [0.0, 0.01]);
    assert_eq!(floats(column(&df, "charge [A·s]")), [0.0, 0.002]);
    assert_eq!(floats(column(&df, "power [W]")), [0.3, 0.6]);
    let unrelated = TempFile::new("csv", b"Time,Power,Temp\n0,1,2\n1,3,4\n");
    let df = frame(open(&unrelated).as_ref());
    assert!(df.get("Power").is_some());
    assert!(df.get("Temp").is_some());
}

#[test]
fn mixed_column_reconstruction_skips_repeated_headers_and_sensor_groups() {
    let mut text = String::from("Date,Time,Value\n");
    for i in 0..70 {
        text.push_str(&format!("03/18/2023,12:00:{:02},{i}\n", i % 60));
    }
    text.push_str("Date,Time,Value\nSensor Group: CPU,ignored,ignored\n");
    text.push_str("03/18/2023,12:01:10,not numeric\n");
    let fixture = TempFile::new("csv", text.as_bytes());
    let df = frame(open(&fixture).as_ref());
    assert_eq!(df.nrows(), 71);
    let values = strings(column(&df, "Value"));
    assert_eq!(values[0], "0");
    assert_eq!(values[69], "69");
    assert_eq!(values[70], "not numeric");
}
