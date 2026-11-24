//! KMS for the splash, the goodbye and the safe mode: the first two connected outputs of the
//! first card, each given a dumb buffer with a picture in it. The DRM ioctls, as in DSlot's
//! kms.rs.

use std::ffi::CString;
use std::os::raw::c_ulong;

/// One screen's picture: XRGB8888 rows, `w` by `h`.
pub struct Screen {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

const fn iow(nr: u32, size: usize) -> c_ulong {
    ((1u32 << 30) | ((size as u32) << 16) | (0x64 << 8) | nr) as c_ulong
}

const fn iowr(nr: u32, size: usize) -> c_ulong {
    ((3u32 << 30) | ((size as u32) << 16) | (0x64 << 8) | nr) as c_ulong
}

#[repr(C)]
#[derive(Default)]
struct SetClientCap {
    capability: u64,
    value: u64,
}

#[repr(C)]
#[derive(Default)]
struct GetPlaneRes {
    plane_id_ptr: u64,
    count_planes: u32,
}

#[repr(C)]
#[derive(Default)]
struct GetPlane {
    plane_id: u32,
    crtc_id: u32,
    fb_id: u32,
    possible_crtcs: u32,
    gamma_size: u32,
    count_format_types: u32,
    format_type_ptr: u64,
}

#[repr(C)]
#[derive(Default)]
struct ObjGetProperties {
    props_ptr: u64,
    prop_values_ptr: u64,
    count_props: u32,
    obj_id: u32,
    obj_type: u32,
}

#[repr(C)]
#[derive(Default)]
struct GetProperty {
    values_ptr: u64,
    enum_blob_ptr: u64,
    prop_id: u32,
    flags: u32,
    name: [u8; 32],
    count_values: u32,
    count_enum_blobs: u32,
}

#[repr(C)]
#[derive(Default)]
struct CreateBlob {
    data: u64,
    length: u32,
    blob_id: u32,
}

#[repr(C)]
#[derive(Default)]
struct DestroyBlob {
    blob_id: u32,
}

#[repr(C)]
#[derive(Default)]
struct Atomic {
    flags: u32,
    count_objs: u32,
    objs_ptr: u64,
    count_props_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    reserved: u64,
    user_data: u64,
}

#[repr(C)]
#[derive(Default)]
struct CardRes {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct ModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    kind: u32,
    name: [u8; 32],
}

#[repr(C)]
#[derive(Default)]
struct GetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encoders: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Default)]
struct GetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[repr(C)]
#[derive(Default)]
struct Crtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: ModeInfo,
}

#[repr(C)]
#[derive(Default)]
struct CreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[repr(C)]
#[derive(Default)]
struct MapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[repr(C)]
#[derive(Default)]
struct FbCmd {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
}

const SET_CLIENT_CAP: c_ulong = iow(0x0D, std::mem::size_of::<SetClientCap>());
const GETRESOURCES: c_ulong = iowr(0xA0, std::mem::size_of::<CardRes>());
const GETCRTC: c_ulong = iowr(0xA1, std::mem::size_of::<Crtc>());
const SETCRTC: c_ulong = iowr(0xA2, std::mem::size_of::<Crtc>());
const GETENCODER: c_ulong = iowr(0xA6, std::mem::size_of::<GetEncoder>());
const GETCONNECTOR: c_ulong = iowr(0xA7, std::mem::size_of::<GetConnector>());
const GETPROPERTY: c_ulong = iowr(0xAA, std::mem::size_of::<GetProperty>());
const ADDFB: c_ulong = iowr(0xAE, std::mem::size_of::<FbCmd>());
const CREATE_DUMB: c_ulong = iowr(0xB2, std::mem::size_of::<CreateDumb>());
const MAP_DUMB: c_ulong = iowr(0xB3, std::mem::size_of::<MapDumb>());
const GETPLANERESOURCES: c_ulong = iowr(0xB5, std::mem::size_of::<GetPlaneRes>());
const GETPLANE: c_ulong = iowr(0xB6, std::mem::size_of::<GetPlane>());
const OBJ_GETPROPERTIES: c_ulong = iowr(0xB9, std::mem::size_of::<ObjGetProperties>());
const ATOMIC: c_ulong = iowr(0xBC, std::mem::size_of::<Atomic>());
const CREATEPROPBLOB: c_ulong = iowr(0xBD, std::mem::size_of::<CreateBlob>());
const DESTROYPROPBLOB: c_ulong = iowr(0xBE, std::mem::size_of::<DestroyBlob>());
const DROP_MASTER: c_ulong = (0x64 << 8) | 0x1f;

const DRM_CLIENT_CAP_UNIVERSAL_PLANES: u64 = 2;
const DRM_CLIENT_CAP_ATOMIC: u64 = 3;
const DRM_MODE_OBJECT_CRTC: u32 = 0xcccc_cccc;
const DRM_MODE_OBJECT_CONNECTOR: u32 = 0xc0c0_c0c0;
const DRM_MODE_OBJECT_PLANE: u32 = 0xeeee_eeee;
const DRM_MODE_ATOMIC_ALLOW_MODESET: u32 = 0x0400;
const DRM_PLANE_TYPE_PRIMARY: u64 = 1;

fn ioctl<T>(fd: i32, req: c_ulong, arg: &mut T) -> Result<(), String> {
    let r = unsafe { libc::ioctl(fd, req as _, arg as *mut T) };
    if r < 0 {
        Err(format!(
            "DRM ioctl {req:#x}: {}",
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

/// A dumb buffer, mapped: XRGB8888 rows, `pitch` bytes apart.
pub struct Buffer {
    pub fb: u32,
    map: *mut u8,
    pitch: usize,
    size: usize,
    w: usize,
    h: usize,
}

impl Buffer {
    /// Copies a picture over its top-left (dumb buffers come zeroed, black).
    pub fn put(&mut self, s: &Screen) {
        let (cw, ch) = (self.w.min(s.w) * 4, self.h.min(s.h));
        for y in 0..ch {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    s.px.as_ptr().add(y * s.w * 4),
                    self.map.add(y * self.pitch),
                    cw,
                )
            };
        }
    }

    // nibos-keys's partial redraws; the splash and the safe mode draw whole pictures.
    #[allow(dead_code)]
    /// Copies the rectangle `(x, y, w, h)` of an RGBA picture `stride` pixels wide to the
    /// same place, as XRGB8888.
    pub fn put_rgba(
        &mut self,
        px: &[u8],
        stride: usize,
        (x, y, w, h): (usize, usize, usize, usize),
    ) {
        let (x1, y1) = ((x + w).min(self.w).min(stride), (y + h).min(self.h));
        if x >= x1 {
            return;
        }
        for row in y..y1 {
            let src = &px[(row * stride + x) * 4..(row * stride + x1) * 4];
            // SAFETY: row < self.h and x1 <= self.w, inside the mapping.
            let dst = unsafe {
                std::slice::from_raw_parts_mut(self.map.add(row * self.pitch + x * 4), (x1 - x) * 4)
            };
            for (d, s) in dst
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src.as_chunks::<4>().0)
            {
                d.copy_from_slice(&[s[2], s[1], s[0], 255]);
            }
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.map.cast(), self.size) };
    }
}

pub struct Output {
    pub crtc: u32,
    pub crtc_index: u32,
    pub connector: u32,
    pub mode: ModeInfo,
}

pub struct Kms {
    pub fd: i32,
    /// Top screen first: the first connected output (DSI-1) is the bottom, touch panel.
    pub outputs: Vec<Output>,
}

impl Kms {
    /// The first two connected outputs of the first KMS card, on the CRTCs the console lit them
    /// with, or the first free CRTC each may use when it hasn't yet.
    pub fn open() -> Result<Kms, String> {
        let mut found = None;
        for _ in 0..40 {
            found = (0..4).find_map(|i| {
                let path = CString::new(format!("/dev/dri/card{i}")).unwrap();
                let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
                if fd < 0 {
                    return None;
                }
                let mut res = CardRes::default();
                if ioctl(fd, GETRESOURCES, &mut res).is_err() || res.count_connectors == 0 {
                    unsafe { libc::close(fd) };
                    return None;
                }
                let mut conns = vec![0u32; res.count_connectors as usize];
                let mut crtcs = vec![0u32; res.count_crtcs as usize];
                let mut res2 = CardRes {
                    connector_id_ptr: conns.as_mut_ptr() as u64,
                    count_connectors: res.count_connectors,
                    crtc_id_ptr: crtcs.as_mut_ptr() as u64,
                    count_crtcs: res.count_crtcs,
                    ..Default::default()
                };
                ioctl(fd, GETRESOURCES, &mut res2).ok()?;
                Some((fd, crtcs, conns))
            });
            if found.is_some() {
                break;
            }
            unsafe { libc::usleep(5_000) };
        }
        let (fd, crtcs, connectors) = found.ok_or("no KMS card")?;
        let mut outputs: Vec<Output> = Vec::new();
        for id in connectors {
            let mut c = GetConnector {
                connector_id: id,
                ..Default::default()
            };
            ioctl(fd, GETCONNECTOR, &mut c)?;
            if c.connection != 1 || c.count_modes == 0 {
                continue;
            }
            let mut modes = vec![ModeInfo::default(); c.count_modes as usize];
            let mut encs = vec![0u32; c.count_encoders as usize];
            let mut c2 = GetConnector {
                connector_id: id,
                modes_ptr: modes.as_mut_ptr() as u64,
                count_modes: c.count_modes,
                encoders_ptr: encs.as_mut_ptr() as u64,
                count_encoders: c.count_encoders,
                ..Default::default()
            };
            ioctl(fd, GETCONNECTOR, &mut c2)?;
            let enc = if c2.encoder_id != 0 {
                c2.encoder_id
            } else {
                encs.first().copied().unwrap_or(0)
            };
            let mut e = GetEncoder {
                encoder_id: enc,
                ..Default::default()
            };
            ioctl(fd, GETENCODER, &mut e)?;
            let free = |crtc: &u32| !outputs.iter().any(|o| o.crtc == *crtc);
            let (crtc_index, crtc) = if e.crtc_id != 0 && free(&e.crtc_id) {
                let idx = crtcs.iter().position(|&c| c == e.crtc_id).unwrap_or(0) as u32;
                (idx, e.crtc_id)
            } else {
                match crtcs
                    .iter()
                    .enumerate()
                    .find(|(i, c)| e.possible_crtcs & (1 << i) != 0 && free(c))
                {
                    Some((i, c)) => (i as u32, *c),
                    None => continue,
                }
            };
            outputs.push(Output {
                crtc,
                crtc_index,
                connector: id,
                mode: modes[0],
            });
            if outputs.len() == 2 {
                break;
            }
        }
        if outputs.is_empty() {
            return Err("no connected output".into());
        }
        outputs.reverse();
        Ok(Kms { fd, outputs })
    }

    /// Each picture into a dumb buffer of its own, set on its output. Returns how many.
    /// With two outputs and deferred fbcon takeover (neither panel lit yet), commits all
    /// outputs in one atomic modeset so DRM prepares both panels in parallel before enabling
    /// them (saving one panel's sleep-out wait); falls back to legacy SETCRTC if anything fails.
    #[allow(dead_code)]
    pub fn show(&self, screens: &[Screen]) -> Result<usize, String> {
        let mut pairs: Vec<(&Output, Buffer)> = Vec::new();
        for (o, s) in self.outputs.iter().zip(screens) {
            let mut b = self.buffer(o)?;
            b.put(s);
            pairs.push((o, b));
        }
        if pairs.len() > 1 && self.show_atomic(&pairs).is_ok() {
            return Ok(pairs.len());
        }
        for (o, b) in &pairs {
            self.set(o, b)?;
        }
        Ok(pairs.len())
    }

    #[allow(dead_code)]
    fn prop_id(&self, obj_id: u32, obj_type: u32, wanted: &str) -> Option<(u32, u64)> {
        let mut gp = ObjGetProperties {
            obj_id,
            obj_type,
            ..Default::default()
        };
        ioctl(self.fd, OBJ_GETPROPERTIES, &mut gp).ok()?;
        let n = gp.count_props as usize;
        if n == 0 {
            return None;
        }
        let mut ids = vec![0u32; n];
        let mut vals = vec![0u64; n];
        gp.props_ptr = ids.as_mut_ptr() as u64;
        gp.prop_values_ptr = vals.as_mut_ptr() as u64;
        ioctl(self.fd, OBJ_GETPROPERTIES, &mut gp).ok()?;
        for (id, val) in ids.into_iter().zip(vals) {
            let mut prop = GetProperty {
                prop_id: id,
                ..Default::default()
            };
            if ioctl(self.fd, GETPROPERTY, &mut prop).is_ok() {
                let len = prop
                    .name
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(prop.name.len());
                if &prop.name[..len] == wanted.as_bytes() {
                    return Some((id, val));
                }
            }
        }
        None
    }

    #[allow(dead_code)]
    fn show_atomic(&self, pairs: &[(&Output, Buffer)]) -> Result<(), String> {
        let mut cap = SetClientCap {
            capability: DRM_CLIENT_CAP_UNIVERSAL_PLANES,
            value: 1,
        };
        ioctl(self.fd, SET_CLIENT_CAP, &mut cap)?;
        cap = SetClientCap {
            capability: DRM_CLIENT_CAP_ATOMIC,
            value: 1,
        };
        ioctl(self.fd, SET_CLIENT_CAP, &mut cap)?;

        let mut pres = GetPlaneRes::default();
        ioctl(self.fd, GETPLANERESOURCES, &mut pres)?;
        if pres.count_planes == 0 {
            return Err("no planes".into());
        }
        let mut plane_ids = vec![0u32; pres.count_planes as usize];
        pres.plane_id_ptr = plane_ids.as_mut_ptr() as u64;
        ioctl(self.fd, GETPLANERESOURCES, &mut pres)?;

        let mut planes: Vec<(u32, u32)> = Vec::new();
        for pid in plane_ids {
            let mut p = GetPlane {
                plane_id: pid,
                ..Default::default()
            };
            if ioctl(self.fd, GETPLANE, &mut p).is_err() {
                continue;
            }
            if let Some((_, kind)) = self.prop_id(pid, DRM_MODE_OBJECT_PLANE, "type") {
                if kind == DRM_PLANE_TYPE_PRIMARY {
                    planes.push((pid, p.possible_crtcs));
                }
            }
        }

        let mut blobs: Vec<u32> = Vec::new();
        let result: Result<(), String> = (|| {
            let mut objs: Vec<u32> = Vec::new();
            let mut counts: Vec<u32> = Vec::new();
            let mut props: Vec<u32> = Vec::new();
            let mut values: Vec<u64> = Vec::new();
            let mut used_planes: Vec<u32> = Vec::new();

            for (o, b) in pairs {
                let &(plane, _) = planes
                    .iter()
                    .find(|&&(pid, mask)| {
                        mask & (1 << o.crtc_index) != 0 && !used_planes.contains(&pid)
                    })
                    .ok_or("no primary plane for CRTC")?;
                used_planes.push(plane);

                let mut blob = CreateBlob {
                    data: &o.mode as *const ModeInfo as u64,
                    length: std::mem::size_of::<ModeInfo>() as u32,
                    blob_id: 0,
                };
                ioctl(self.fd, CREATEPROPBLOB, &mut blob)?;
                blobs.push(blob.blob_id);

                let conn_crtc = self
                    .prop_id(o.connector, DRM_MODE_OBJECT_CONNECTOR, "CRTC_ID")
                    .ok_or("no connector CRTC_ID")?
                    .0;
                let crtc_active = self
                    .prop_id(o.crtc, DRM_MODE_OBJECT_CRTC, "ACTIVE")
                    .ok_or("no CRTC ACTIVE")?
                    .0;
                let crtc_mode = self
                    .prop_id(o.crtc, DRM_MODE_OBJECT_CRTC, "MODE_ID")
                    .ok_or("no CRTC MODE_ID")?
                    .0;
                let pp = |name| {
                    self.prop_id(plane, DRM_MODE_OBJECT_PLANE, name)
                        .map(|(id, _)| id)
                        .ok_or_else(|| format!("no plane {name}"))
                };
                let (w, h) = (o.mode.hdisplay as u64, o.mode.vdisplay as u64);
                let mut add = |obj: u32, pairs: &[(u32, u64)]| {
                    objs.push(obj);
                    counts.push(pairs.len() as u32);
                    for &(p, v) in pairs {
                        props.push(p);
                        values.push(v);
                    }
                };
                add(o.connector, &[(conn_crtc, o.crtc as u64)]);
                add(
                    o.crtc,
                    &[(crtc_active, 1), (crtc_mode, blob.blob_id as u64)],
                );
                add(
                    plane,
                    &[
                        (pp("FB_ID")?, b.fb as u64),
                        (pp("CRTC_ID")?, o.crtc as u64),
                        (pp("SRC_X")?, 0),
                        (pp("SRC_Y")?, 0),
                        (pp("SRC_W")?, w << 16),
                        (pp("SRC_H")?, h << 16),
                        (pp("CRTC_X")?, 0),
                        (pp("CRTC_Y")?, 0),
                        (pp("CRTC_W")?, w),
                        (pp("CRTC_H")?, h),
                    ],
                );
            }
            let mut commit = Atomic {
                flags: DRM_MODE_ATOMIC_ALLOW_MODESET,
                count_objs: objs.len() as u32,
                objs_ptr: objs.as_ptr() as u64,
                count_props_ptr: counts.as_ptr() as u64,
                props_ptr: props.as_ptr() as u64,
                prop_values_ptr: values.as_ptr() as u64,
                reserved: 0,
                user_data: 0,
            };
            ioctl(self.fd, ATOMIC, &mut commit)
        })();

        for blob_id in blobs {
            let mut d = DestroyBlob { blob_id };
            let _ = ioctl(self.fd, DESTROYPROPBLOB, &mut d);
        }
        result
    }

    /// A dumb buffer the size of `o`'s mode, black, mapped for writing. It stays on the card
    /// once set; dropping it only unmaps it.
    pub fn buffer(&self, o: &Output) -> Result<Buffer, String> {
        let fd = self.fd;
        let (w, h) = (o.mode.hdisplay as u32, o.mode.vdisplay as u32);
        let mut d = CreateDumb {
            width: w,
            height: h,
            bpp: 32,
            ..Default::default()
        };
        ioctl(fd, CREATE_DUMB, &mut d)?;
        let mut f = FbCmd {
            width: w,
            height: h,
            pitch: d.pitch,
            bpp: 32,
            depth: 24,
            handle: d.handle,
            ..Default::default()
        };
        ioctl(fd, ADDFB, &mut f)?;
        let mut m = MapDumb {
            handle: d.handle,
            ..Default::default()
        };
        ioctl(fd, MAP_DUMB, &mut m)?;
        let map = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                d.size as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                m.offset as libc::off_t,
            )
        };
        if map == libc::MAP_FAILED {
            return Err("mmap of a dumb buffer failed".into());
        }
        Ok(Buffer {
            fb: f.fb_id,
            map: map.cast(),
            pitch: d.pitch as usize,
            size: d.size as usize,
            w: w as usize,
            h: h as usize,
        })
    }

    /// Puts `b` on output `o` (once: what is written to it after shows straight away).
    pub fn set(&self, o: &Output, b: &Buffer) -> Result<(), String> {
        let mut conn = o.connector;
        let mut c = Crtc {
            set_connectors_ptr: &mut conn as *mut u32 as u64,
            count_connectors: 1,
            crtc_id: o.crtc,
            fb_id: b.fb,
            mode_valid: 1,
            mode: o.mode,
            ..Default::default()
        };
        ioctl(self.fd, SETCRTC, &mut c)
    }

    pub fn drop_master(&self) {
        unsafe { libc::ioctl(self.fd, DROP_MASTER as _) };
    }

    pub fn fb(&self, crtc: u32) -> u32 {
        let mut c = Crtc {
            crtc_id: crtc,
            ..Default::default()
        };
        if ioctl(self.fd, GETCRTC, &mut c).is_err() {
            0
        } else {
            c.fb_id
        }
    }
}
