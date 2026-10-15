//! TurbSim BTS velocity boxes, tower data, statistics, planes and binary export.
use super::binary::{broken, frame, BinaryReader};
use dv_core::{DataFrame, Field2D, FileObject, IoError, NamedFrame};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
#[derive(Debug, Clone)]
pub struct TurbSimOptions {
    pub header_only: bool,
    pub time_decimals: u32,
}
impl Default for TurbSimOptions {
    fn default() -> Self {
        Self {
            header_only: false,
            time_decimals: 8,
        }
    }
}
#[derive(Debug, Clone)]
pub struct TurbSimFile {
    pub filename: PathBuf,
    pub id: i16,
    pub description: String,
    pub y: Vec<f64>,
    pub z: Vec<f64>,
    pub t: Vec<f64>,
    pub tower_z: Vec<f64>,
    pub dt: f64,
    pub reference_height: f64,
    pub reference_speed: f64,
    /// Component-major, then time, lateral position, height: (3, nt, ny, nz).
    pub velocity: Vec<f64>,
    /// Component-major, then time and tower point: (3, nt, nTower).
    pub tower_velocity: Vec<f64>,
}
fn nearest(a: &[f64], v: f64) -> usize {
    a.iter()
        .enumerate()
        .min_by(|(_, x), (_, y)| (*x - v).abs().total_cmp(&(*y - v).abs()))
        .map_or(0, |(i, _)| i)
}
fn mean(a: &[f64]) -> f64 {
    a.iter().sum::<f64>() / a.len() as f64
}
fn stddev(a: &[f64]) -> f64 {
    let m = mean(a);
    (a.iter().map(|v| (v - m).powi(2)).sum::<f64>() / a.len() as f64).sqrt()
}
impl TurbSimFile {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        Self::read_with_options(path, &Default::default())
    }
    pub fn read_with_options(path: &Path, options: &TurbSimOptions) -> Result<Self, IoError> {
        let mut r = BinaryReader(BufReader::new(File::open(path)?));
        let id = r.i16()?;
        if !matches!(id, 7 | 8) {
            return Err(IoError::WrongFormat(format!("not a TurbSim BTS ID: {id}")));
        }
        let nz = r.count("vertical count")?;
        let ny = r.count("lateral count")?;
        let nw = r.count("tower count")?;
        let nt = r.count("time count")?;
        if nz == 0 || ny == 0 || nt == 0 {
            return Err(broken("TurbSim dimensions must be nonzero"));
        }
        let dz = r.f32()? as f64;
        let dy = r.f32()? as f64;
        let dt = r.f32()? as f64;
        let reference_speed = r.f32()? as f64;
        let reference_height = r.f32()? as f64;
        let bottom = r.f32()? as f64;
        let mut scales = [0.0; 3];
        let mut offsets = [0.0; 3];
        for k in 0..3 {
            scales[k] = r.f32()? as f64;
            offsets[k] = r.f32()? as f64;
        }
        let nchar = r.count("description length")?;
        let description = String::from_utf8_lossy(&r.bytes(nchar)?).into_owned();
        let plane = ny
            .checked_mul(nz)
            .ok_or_else(|| broken("TurbSim dimensions overflow"))?;
        let count = 3usize
            .checked_mul(nt)
            .and_then(|v| v.checked_mul(plane))
            .ok_or_else(|| broken("TurbSim dimensions overflow"))?;
        let tower_count = 3usize
            .checked_mul(nt)
            .and_then(|v| v.checked_mul(nw))
            .ok_or_else(|| broken("TurbSim dimensions overflow"))?;
        if !options.header_only {
            let expected = 70u64
                .checked_add(nchar as u64)
                .and_then(|v| v.checked_add((count as u64 + tower_count as u64).checked_mul(2)?))
                .ok_or_else(|| broken("TurbSim dimensions overflow"))?;
            if std::fs::metadata(path)?.len() < expected {
                return Err(broken("truncated TurbSim velocity field"));
            }
        }
        let mut velocity = if options.header_only {
            Vec::new()
        } else {
            vec![0.0; count]
        };
        let mut tower_velocity = if options.header_only {
            Vec::new()
        } else {
            vec![0.0; tower_count]
        };
        if !options.header_only {
            for it in 0..nt {
                for iz in 0..nz {
                    for iy in 0..ny {
                        for k in 0..3 {
                            velocity[((k * nt + it) * ny + iy) * nz + iz] =
                                (r.i16()? as f64 - offsets[k]) / scales[k];
                        }
                    }
                }
                for iw in 0..nw {
                    for k in 0..3 {
                        tower_velocity[(k * nt + it) * nw + iw] =
                            (r.i16()? as f64 - offsets[k]) / scales[k];
                    }
                }
            }
        }
        let decimals = 10.0f64.powi(options.time_decimals.min(15) as i32);
        let round = |v: f64| (v * decimals).round_ties_even() / decimals;
        Ok(Self {
            filename: path.into(),
            id,
            description,
            y: (0..ny)
                .map(|i| (i as f64 - (ny - 1) as f64 / 2.0) * dy)
                .collect(),
            z: (0..nz).map(|i| bottom + i as f64 * dz).collect(),
            t: (0..nt).map(|i| round(i as f64 * dt)).collect(),
            tower_z: (0..nw).map(|i| bottom - i as f64 * dz).collect(),
            dt: round(dt),
            reference_height,
            reference_speed,
            velocity,
            tower_velocity,
        })
    }
    fn index(&self, k: usize, it: usize, iy: usize, iz: usize) -> usize {
        ((k * self.t.len() + it) * self.y.len() + iy) * self.z.len() + iz
    }
    pub fn value(&self, k: usize, it: usize, iy: usize, iz: usize) -> f64 {
        self.velocity[self.index(k, it, iy, iz)]
    }
    fn require_field(&self) -> Result<(), IoError> {
        let n = 3 * self.t.len() * self.y.len() * self.z.len();
        if self.velocity.len() != n || self.t.is_empty() || self.y.is_empty() || self.z.is_empty() {
            Err(broken("TurbSim full velocity field is required"))
        } else {
            Ok(())
        }
    }
    pub fn closest_point(&self, y: f64, z: f64) -> (usize, usize) {
        (nearest(&self.y, y), nearest(&self.z, z))
    }
    pub fn mid_indices(&self) -> (usize, usize) {
        self.closest_point(
            (self.y[0] + self.y[self.y.len() - 1]) / 2.0,
            (self.z[0] + self.z[self.z.len() - 1]) / 2.0,
        )
    }
    pub fn values_at(&self, y: f64, z: f64) -> Result<[Vec<f64>; 3], IoError> {
        self.require_field()?;
        let (iy, iz) = self.closest_point(y, z);
        Ok(self.line(iy, iz))
    }
    fn line(&self, iy: usize, iz: usize) -> [Vec<f64>; 3] {
        std::array::from_fn(|k| {
            (0..self.t.len())
                .map(|t| self.value(k, t, iy, iz))
                .collect()
        })
    }
    pub fn vertical_profile(
        &self,
        mid_only: bool,
    ) -> Result<([Vec<f64>; 3], [Vec<f64>; 3]), IoError> {
        self.require_field()?;
        let mut means = std::array::from_fn(|_| vec![0.0; self.z.len()]);
        let mut stds = std::array::from_fn(|_| vec![0.0; self.z.len()]);
        let iy = if mid_only {
            vec![self.mid_indices().0]
        } else {
            (0..self.y.len()).collect()
        };
        for &y in &iy {
            for z in 0..self.z.len() {
                let lines = self.line(y, z);
                for k in 0..3 {
                    means[k][z] += mean(&lines[k]) / iy.len() as f64;
                    stds[k][z] += stddev(&lines[k]) / iy.len() as f64;
                }
            }
        }
        Ok((means, stds))
    }
    pub fn horizontal_plane(&self, z: f64, remove_mean: bool) -> Result<[Vec<f64>; 3], IoError> {
        self.require_field()?;
        let iz = nearest(&self.z, z);
        let mut out = std::array::from_fn(|k| {
            (0..self.t.len())
                .flat_map(|t| (0..self.y.len()).map(move |y| self.value(k, t, y, iz)))
                .collect::<Vec<_>>()
        });
        if remove_mean {
            for a in &mut out {
                let m = mean(a);
                for v in a {
                    *v -= m;
                }
            }
        }
        Ok(out)
    }
    pub fn vertical_plane(&self, y: f64, remove_mean: bool) -> Result<[Vec<f64>; 3], IoError> {
        self.require_field()?;
        let iy = nearest(&self.y, y);
        let mut out = std::array::from_fn(|k| {
            (0..self.t.len())
                .flat_map(|t| (0..self.z.len()).map(move |z| self.value(k, t, iy, z)))
                .collect::<Vec<_>>()
        });
        if remove_mean {
            for a in &mut out {
                let m = mean(a);
                for v in a {
                    *v -= m;
                }
            }
        }
        Ok(out)
    }
    pub fn normal_plane(&self, time: f64, remove_mean: bool) -> Result<[Vec<f64>; 3], IoError> {
        self.require_field()?;
        let t = nearest(&self.t, time);
        let mut out = std::array::from_fn(|k| {
            (0..self.y.len())
                .flat_map(|y| (0..self.z.len()).map(move |z| self.value(k, t, y, z)))
                .collect::<Vec<_>>()
        });
        if remove_mean {
            for a in &mut out {
                let m = mean(a);
                for v in a {
                    *v -= m;
                }
            }
        }
        Ok(out)
    }
    pub fn cross_correlation(
        &self,
        along_y: bool,
        reference: Option<(usize, usize)>,
    ) -> Result<[Vec<f64>; 3], IoError> {
        self.require_field()?;
        let (iy, iz) = reference.unwrap_or_else(|| self.mid_indices());
        if iy >= self.y.len() || iz >= self.z.len() {
            return Err(broken("correlation reference outside grid"));
        }
        let base = self.line(iy, iz);
        let n = if along_y { self.y.len() } else { self.z.len() };
        let mut out = std::array::from_fn(|_| vec![0.0; n]);
        for i in 0..n {
            let other = if along_y {
                self.line(i, iz)
            } else {
                self.line(iy, i)
            };
            for k in 0..3 {
                let a = &base[k];
                let b = &other[k];
                let ma = mean(a);
                let mb = mean(b);
                out[k][i] = a
                    .iter()
                    .zip(b)
                    .map(|(x, y)| (x - ma) * (y - mb))
                    .sum::<f64>()
                    / a.len() as f64
                    / (stddev(a) * stddev(b));
            }
        }
        Ok(out)
    }
    pub fn scale(
        &mut self,
        component: usize,
        new_mean: Option<f64>,
        new_std: Option<f64>,
        point: Option<(f64, f64)>,
    ) -> Result<(), IoError> {
        self.require_field()?;
        if component >= 3 {
            return Err(broken("velocity component must be 0,1,2"));
        }
        let (iy, iz) = point
            .map(|(y, z)| self.closest_point(y, z))
            .unwrap_or_else(|| self.mid_indices());
        let line = self.line(iy, iz);
        let old_mean = mean(&line[component]);
        let old_std = stddev(&line[component]);
        for y in 0..self.y.len() {
            for z in 0..self.z.len() {
                let local = self.line(y, z);
                let m = mean(&local[component]);
                for t in 0..self.t.len() {
                    let i = self.index(component, t, y, z);
                    self.velocity[i] = (self.velocity[i] - m)
                        * new_std.map_or(1.0, |s| s / old_std)
                        + m
                        + new_mean.map_or(0.0, |v| v - old_mean);
                }
            }
        }
        Ok(())
    }
    pub fn make_periodic(&mut self) -> Result<(), IoError> {
        self.require_field()?;
        let old_nt = self.t.len();
        if old_nt < 2 {
            return Err(broken("periodic box requires two time samples"));
        }
        let nt = 2 * old_nt - 2;
        let ny = self.y.len();
        let nz = self.z.len();
        let nw = self.tower_z.len();
        let mut v = vec![0.0; 3 * nt * ny * nz];
        let mut tower = vec![0.0; 3 * nt * nw];
        for k in 0..3 {
            for t in 0..nt {
                let source = if t < old_nt { t } else { 2 * old_nt - t - 2 };
                for y in 0..ny {
                    for z in 0..nz {
                        v[((k * nt + t) * ny + y) * nz + z] = self.value(k, source, y, z);
                    }
                }
                for w in 0..nw {
                    tower[(k * nt + t) * nw + w] =
                        self.tower_velocity[(k * old_nt + source) * nw + w];
                }
            }
        }
        let dt = self.t[1] - self.t[0];
        self.velocity = v;
        self.tower_velocity = tower;
        self.t = (0..nt).map(|t| t as f64 * dt).collect();
        self.id = 8;
        Ok(())
    }
    pub fn check_periodic(
        &self,
        sigma_tolerance: f64,
        absolute_tolerance: f64,
    ) -> Result<bool, IoError> {
        self.require_field()?;
        let mut max_relative = 0.0f64;
        let mut sum = 0.0;
        for y in 0..self.y.len() {
            for z in 0..self.z.len() {
                let line = self.line(y, z);
                let difference = (line[0][0] - line[0][line[0].len() - 1]).abs();
                let relative = difference / stddev(&line[0]);
                if relative.is_nan() {
                    return Ok(false);
                }
                max_relative = max_relative.max(relative);
                sum += difference;
            }
        }
        Ok(max_relative < sigma_tolerance
            && sum / ((self.y.len() * self.z.len()) as f64) < absolute_tolerance)
    }
    pub fn write(&self, path: &Path) -> Result<(), IoError> {
        self.require_field()?;
        let nt = self.t.len();
        let ny = self.y.len();
        let nz = self.z.len();
        let nw = self.tower_z.len();
        if nt < 2 || ny < 2 || nz < 2 {
            return Err(broken(
                "BTS export requires at least two points per coordinate",
            ));
        }
        if self.tower_velocity.len() != 3 * nt * nw {
            return Err(broken("tower velocity dimensions differ"));
        }
        let mut scales = [0.0f32; 3];
        let mut offsets = [0.0f32; 3];
        let block = nt * ny * nz;
        for k in 0..3 {
            let values = self.velocity[k * block..(k + 1) * block]
                .iter()
                .chain(&self.tower_velocity[k * nt * nw..(k + 1) * nt * nw]);
            let mut lo = f64::INFINITY;
            let mut hi = f64::NEG_INFINITY;
            for &v in values {
                if !v.is_finite() {
                    return Err(broken("BTS export cannot represent non-finite velocities"));
                }
                lo = lo.min(v);
                hi = hi.max(v);
            }
            scales[k] = if hi == lo {
                1.0
            } else {
                (65535.0 / (hi - lo)) as f32
            };
            offsets[k] = (-32768.0 - scales[k] as f64 * lo) as f32;
        }
        let mut w = BufWriter::new(File::create(path)?);
        w.write_all(&self.id.to_le_bytes())?;
        for n in [nz, ny, nw, nt] {
            w.write_all(
                &i32::try_from(n)
                    .map_err(|_| broken("BTS dimension exceeds i32"))?
                    .to_le_bytes(),
            )?;
        }
        for v in [
            self.z[1] - self.z[0],
            self.y[1] - self.y[0],
            self.t[1] - self.t[0],
            self.reference_speed,
            self.reference_height,
            self.z[0],
        ] {
            w.write_all(&(v as f32).to_le_bytes())?;
        }
        for k in 0..3 {
            w.write_all(&scales[k].to_le_bytes())?;
            w.write_all(&offsets[k].to_le_bytes())?;
        }
        let info = "Generated by datview TurbSimFile.";
        w.write_all(&(info.len() as i32).to_le_bytes())?;
        w.write_all(info.as_bytes())?;
        for t in 0..nt {
            for z in 0..nz {
                for y in 0..ny {
                    for k in 0..3 {
                        let packed =
                            (self.value(k, t, y, z) * scales[k] as f64 + offsets[k] as f64) as i16;
                        w.write_all(&packed.to_le_bytes())?;
                    }
                }
            }
            for p in 0..nw {
                for k in 0..3 {
                    let packed = (self.tower_velocity[(k * nt + t) * nw + p] * scales[k] as f64
                        + offsets[k] as f64) as i16;
                    w.write_all(&packed.to_le_bytes())?;
                }
            }
        }
        w.flush()?;
        Ok(())
    }
    pub fn fields(&self, n_time: usize, n_y: usize, n_z: usize) -> Result<Vec<Field2D>, IoError> {
        self.require_field()?;
        let nt = self.t.len();
        let ny = self.y.len();
        let nz = self.z.len();
        let mut fields = Vec::new();
        let sample = |n: usize, count: usize| -> Vec<usize> {
            let mut indices = (0..count)
                .map(|i| {
                    if count <= 1 {
                        0
                    } else {
                        i * (n - 1) / (count - 1)
                    }
                })
                .collect::<Vec<_>>();
            indices.dedup();
            indices
        };
        let iy = sample(ny, n_y);
        let iz = sample(nz, n_z);
        let mut it = (0..n_time.min(nt)).collect::<Vec<_>>();
        if !it.contains(&(nt - 1)) {
            it.push(nt - 1);
        }
        let mut push = |name: String, x: &[f64], y: &[f64], sx: &str, sy: &str, m: Vec<f64>| {
            fields.push(Field2D {
                name,
                x: x.to_vec(),
                y: y.to_vec(),
                sx: sx.into(),
                sy: sy.into(),
                m,
            });
        };
        for (k, c) in ["u", "v", "w"].iter().enumerate() {
            let yz = (0..ny)
                .flat_map(|y| {
                    (0..nz).map(move |z| {
                        (0..nt).map(|t| self.value(k, t, y, z)).sum::<f64>() / nt as f64
                    })
                })
                .collect();
            push(
                format!("(y,z)_{c}_avg_[m/s]"),
                &self.y,
                &self.z,
                "y [m]",
                "z [m]",
                yz,
            );
        }
        for (k, c) in ["u", "v", "w"].iter().enumerate() {
            let ty = (0..nt)
                .flat_map(|t| {
                    (0..ny).map(move |y| {
                        (0..nz).map(|z| self.value(k, t, y, z)).sum::<f64>() / nz as f64
                    })
                })
                .collect();
            push(
                format!("(t,y)_{c}_avg_[m/s]"),
                &self.t,
                &self.y,
                "t [s]",
                "y [m]",
                ty,
            );
        }
        for (k, c) in ["u", "v", "w"].iter().enumerate() {
            let tz = (0..nt)
                .flat_map(|t| {
                    (0..nz).map(move |z| {
                        (0..ny).map(|y| self.value(k, t, y, z)).sum::<f64>() / ny as f64
                    })
                })
                .collect();
            push(
                format!("(t,z)_{c}_avg_[m/s]"),
                &self.t,
                &self.z,
                "t [s]",
                "z [m]",
                tz,
            );
        }
        for (k, c) in ["u", "v", "w"].iter().enumerate() {
            for &t in &it {
                let m = (0..ny)
                    .flat_map(|y| (0..nz).map(move |z| self.value(k, t, y, z)))
                    .collect();
                push(
                    format!("(y,z)_{c}_t={:.2}_[m/s]", self.t[t]),
                    &self.y,
                    &self.z,
                    "y [m]",
                    "z [m]",
                    m,
                );
            }
        }
        for (k, c) in ["u", "v", "w"].iter().enumerate() {
            for &y in &iy {
                let m = (0..nt)
                    .flat_map(|t| (0..nz).map(move |z| self.value(k, t, y, z)))
                    .collect();
                push(
                    format!("(t,z)_{c}_y={:.2}_[m/s]", self.y[y]),
                    &self.t,
                    &self.z,
                    "t [s]",
                    "z [m]",
                    m,
                );
            }
        }
        for (k, c) in ["u", "v", "w"].iter().enumerate() {
            for &z in &iz {
                let m = (0..nt)
                    .flat_map(|t| (0..ny).map(move |y| self.value(k, t, y, z)))
                    .collect();
                push(
                    format!("(t,y)_{c}_z={:.2}_[m/s]", self.z[z]),
                    &self.t,
                    &self.y,
                    "t [s]",
                    "y [m]",
                    m,
                );
            }
        }
        Ok(fields)
    }
    pub fn hub_values(&self, height: Option<f64>) -> Result<(f64, f64, bool), IoError> {
        self.require_field()?;
        let known = height.is_some() || self.reference_height.is_finite();
        let z = height.unwrap_or_else(|| {
            if self.reference_height.is_finite() {
                self.reference_height
            } else {
                self.z[self.mid_indices().1]
            }
        });
        let speed = if self.reference_speed.is_finite() {
            self.reference_speed
        } else {
            mean(&self.values_at((self.y[0] + self.y[self.y.len() - 1]) / 2.0, z)?[0])
        };
        Ok((z, speed, known))
    }
    pub fn mid_values(&self) -> Result<(f64, f64), IoError> {
        self.require_field()?;
        let (iy, iz) = self.mid_indices();
        Ok((self.z[iz], mean(&self.line(iy, iz)[0])))
    }
    /// Import component arrays in (time,y,z) order, retaining the Mann -y to +y convention.
    pub fn from_mann_box(
        components: [&[f64]; 3],
        nt: usize,
        y: Vec<f64>,
        z: Vec<f64>,
        dx: f64,
        speed: f64,
        add_u: Option<f64>,
    ) -> Result<Self, IoError> {
        let count = nt
            .checked_mul(y.len())
            .and_then(|n| n.checked_mul(z.len()))
            .ok_or_else(|| broken("Mann dimensions overflow"))?;
        if nt == 0 || y.is_empty() || z.is_empty() || components.iter().any(|c| c.len() != count) {
            return Err(broken("Mann component dimensions differ"));
        }
        if y[0] > y[y.len() - 1] {
            return Err(broken(
                "Mann y coordinates must go from negative to positive",
            ));
        }
        let dt = dx / speed;
        let mut velocity = Vec::with_capacity(3 * count);
        velocity.extend(components[0].iter().map(|v| v + add_u.unwrap_or(0.0)));
        velocity.extend_from_slice(components[1]);
        velocity.extend_from_slice(components[2]);
        let mut out = Self {
            filename: PathBuf::new(),
            id: 7,
            description: "Converted from MannBox fields.".into(),
            y,
            z,
            t: (0..nt).map(|i| i as f64 * dt).collect(),
            tower_z: Vec::new(),
            dt,
            reference_height: f64::NAN,
            reference_speed: f64::NAN,
            velocity,
            tower_velocity: Vec::new(),
        };
        let (height, speed, _) = out.hub_values(None)?;
        out.reference_height = height;
        out.reference_speed = speed;
        Ok(out)
    }
    pub fn to_mann_box(
        &self,
        base: Option<&Path>,
        remove_u_constant: Option<f64>,
        remove_all_u_mean: bool,
    ) -> Result<[PathBuf; 3], IoError> {
        self.require_field()?;
        let base = base
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.filename.with_extension(""));
        let prefix = format!(
            "{}_{}x{}x{}",
            base.display(),
            self.t.len(),
            self.y.len(),
            self.z.len()
        );
        let paths = ["u", "v", "w"].map(|c| PathBuf::from(format!("{prefix}.{c}")));
        for (k, path) in paths.iter().enumerate() {
            let mut w = BufWriter::new(File::create(path)?);
            let means = if k == 0 && remove_all_u_mean {
                Some(
                    (0..self.y.len())
                        .flat_map(|y| {
                            (0..self.z.len()).map(move |z| {
                                (0..self.t.len())
                                    .map(|t| self.value(0, t, y, z))
                                    .sum::<f64>()
                                    / self.t.len() as f64
                            })
                        })
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            };
            for t in 0..self.t.len() {
                for y in (0..self.y.len()).rev() {
                    for z in 0..self.z.len() {
                        let subtract = if let Some(m) = &means {
                            m[y * self.z.len() + z]
                        } else if k == 0 {
                            remove_u_constant.unwrap_or(0.0)
                        } else {
                            0.0
                        };
                        w.write_all(&((self.value(k, t, y, z) - subtract) as f32).to_le_bytes())?;
                    }
                }
            }
            w.flush()?;
        }
        Ok(paths)
    }
    pub fn write_probes(
        &self,
        path: &Path,
        y_probes: &[f64],
        z_probes: &[f64],
    ) -> Result<(), IoError> {
        self.require_field()?;
        let mut names = vec!["Time_[s]".to_owned()];
        let mut cols = vec![self.t.clone()];
        for &y in y_probes {
            for &z in z_probes {
                let (iy, iz) = self.closest_point(y, z);
                for c in ["u", "v", "w"] {
                    names.push(format!("{c}_y{:.0}_z{:.0}_[m/s]", self.y[iy], self.z[iz]));
                }
                cols.extend(self.line(iy, iz));
            }
        }
        let mut w = BufWriter::new(File::create(path)?);
        writeln!(w, "# {}", names.join(","))?;
        for t in 0..self.t.len() {
            for (i, c) in cols.iter().enumerate() {
                if i > 0 {
                    write!(w, ",")?;
                }
                write!(w, "{:.18e}", c[t])?;
            }
            writeln!(w)?;
        }
        w.flush()?;
        Ok(())
    }
    pub fn fit_power_law(
        &self,
        reference_height: Option<f64>,
        mid_only: bool,
        guess: (f64, f64),
    ) -> Result<(dv_tools::curve_fitting::FitResult, f64), IoError> {
        let z_ref =
            reference_height.unwrap_or_else(|| (self.z[0] + self.z[self.z.len() - 1]) / 2.0);
        let (profile, _) = self.vertical_profile(mid_only)?;
        let fit = dv_tools::curve_fitting::fit_powerlaw_u_alpha(&self.z, &profile[0], z_ref, guess)
            .map_err(|e| broken(e.to_string()))?;
        Ok((fit, z_ref))
    }
    pub fn write_info(&self, path: &Path) -> Result<(), IoError> {
        self.require_field()?;
        let (fit, z_ref) = self.fit_power_law(None, true, (10.0, 0.1))?;
        let alpha = fit
            .coefficients
            .iter()
            .find(|(k, _)| k == "alpha")
            .map(|(_, v)| *v)
            .ok_or_else(|| broken("power law fit lacks alpha"))?;
        let speed = fit
            .coefficients
            .iter()
            .find(|(k, _)| k == "u_ref")
            .map(|(_, v)| *v)
            .ok_or_else(|| broken("power law fit lacks u_ref"))?;
        let mut w = BufWriter::new(File::create(path)?);
        writeln!(w, "{}", self.info())?;
        writeln!(w, "Middle height of box: {z_ref:.3}")?;
        writeln!(
            w,
            "Power law: alpha={alpha:.5}  -  u={speed:.5}  at z={z_ref:.5}"
        )?;
        writeln!(w, "Periodic: {}", self.check_periodic(1.5, 0.5)?)?;
        w.flush()?;
        Ok(())
    }
    fn spectra(&self, lines: [Vec<f64>; 3]) -> Result<DataFrame, IoError> {
        use dv_tools::spectral::{welch, WelchOptions};
        let options = WelchOptions {
            fs: 1.0 / (self.t[1] - self.t[0]),
            ..Default::default()
        };
        let mut frequency = None;
        let mut values = Vec::new();
        for line in lines {
            let spectrum = welch(&line, &options).map_err(|e| broken(e.to_string()))?;
            if frequency.is_none() {
                frequency = Some(spectrum.frequency);
            }
            values.push(spectrum.values);
        }
        values.insert(0, frequency.unwrap_or_default());
        frame(
            ["f_[Hz]", "chi_uu_[-]", "chi_vv_[-]", "chi_ww_[-]"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            values,
        )
    }
}
impl FileObject for TurbSimFile {
    fn format_name(&self) -> &str {
        "TurbSim binary"
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        self.require_field()?;
        let mut frames = Vec::new();
        let (iy, iz) = self.mid_indices();
        let (m, s) = self.vertical_profile(false)?;
        let mut cols = vec![self.z.clone()];
        cols.extend(m.iter().cloned());
        cols.extend(s.iter().cloned());
        cols.push(s[0].iter().zip(&m[0]).map(|(s, m)| s / m * 100.0).collect());
        frames.push(NamedFrame::new(
            "VertProfile",
            frame(
                [
                    "z_[m]",
                    "u_[m/s]",
                    "v_[m/s]",
                    "w_[m/s]",
                    "sigma_u_[m/s]",
                    "sigma_v_[m/s]",
                    "sigma_w_[m/s]",
                    "TI_[%]",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                cols,
            )?,
        ));
        for (name, y) in [
            ("ZMidLine", iy),
            ("ZMidYStartLine", 0),
            ("ZMidYEndLine", self.y.len() - 1),
        ] {
            let mut cols = vec![self.t.clone()];
            cols.extend(self.line(y, iz));
            frames.push(NamedFrame::new(
                name,
                frame(
                    ["t_[s]", "u_[m/s]", "v_[m/s]", "w_[m/s]"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    cols,
                )?,
            ));
        }
        for along_y in [true, false] {
            let mut cols = vec![if along_y {
                self.y.clone()
            } else {
                self.z.clone()
            }];
            cols.extend(self.cross_correlation(along_y, None)?);
            let axis = if along_y { "y" } else { "z" };
            frames.push(NamedFrame::new(
                format!("Mid_xcorr_{axis}"),
                frame(
                    vec![
                        format!("{axis}_[m]"),
                        "rho_uu_[-]".into(),
                        "rho_vv_[-]".into(),
                        "rho_ww_[-]".into(),
                    ],
                    cols,
                )?,
            ));
        }
        if self.t.len() < 2 {
            return Err(broken("spectral tables require two time samples"));
        }
        let t = self.t.len() / 2;
        let lateral =
            std::array::from_fn(|k| (0..self.y.len()).map(|y| self.value(k, t, y, iz)).collect());
        let vertical =
            std::array::from_fn(|k| (0..self.z.len()).map(|z| self.value(k, t, iy, z)).collect());
        frames.push(NamedFrame::new(
            "Mid_csd_longi",
            self.spectra(self.line(iy, iz))?,
        ));
        frames.push(NamedFrame::new("Mid_csd_lat", self.spectra(lateral)?));
        frames.push(NamedFrame::new("Mid_csd_vert", self.spectra(vertical)?));
        Ok(frames)
    }
    fn to_2d_fields(&self) -> Option<Result<Vec<Field2D>, IoError>> {
        Some(self.fields(10, 3, 3))
    }
    fn info(&self) -> String {
        format!(
            "TurbSim ID {}: 3 x {} x {} x {} velocity box; {} tower points\n{}",
            self.id,
            self.t.len(),
            self.y.len(),
            self.z.len(),
            self.tower_z.len(),
            self.description
        )
    }
}
