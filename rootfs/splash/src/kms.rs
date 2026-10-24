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

const fn iowr(nr: u32, size: usize) -> c_ulong {
    ((3u32 << 30) | ((size as u32) << 16) | (0x64 << 8) | nr) as c_ulong
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

const GETRESOURCES: c_ulong = iowr(0xA0, std::mem::size_of::<CardRes>());
const GETCRTC: c_ulong = iowr(0xA1, std::mem::size_of::<Crtc>());
const SETCRTC: c_ulong = iowr(0xA2, std::mem::size_of::<Crtc>());
const GETENCODER: c_ulong = iowr(0xA6, std::mem::size_of::<GetEncoder>());
const GETCONNECTOR: c_ulong = iowr(0xA7, std::mem::size_of::<GetConnector>());
const ADDFB: c_ulong = iowr(0xAE, std::mem::size_of::<FbCmd>());
const CREATE_DUMB: c_ulong = iowr(0xB2, std::mem::size_of::<CreateDumb>());
const MAP_DUMB: c_ulong = iowr(0xB3, std::mem::size_of::<MapDumb>());
const DROP_MASTER: c_ulong = (0x64 << 8) | 0x1f;

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
        let (fd, crtcs, connectors) = (0..4)
            .find_map(|i| {
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
            })
            .ok_or("no KMS card")?;
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
            let crtc = if e.crtc_id != 0 && free(&e.crtc_id) {
                e.crtc_id
            } else {
                match crtcs
                    .iter()
                    .enumerate()
                    .find(|(i, c)| e.possible_crtcs & (1 << i) != 0 && free(c))
                {
                    Some((_, c)) => *c,
                    None => continue,
                }
            };
            outputs.push(Output {
                crtc,
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
    pub fn show(&self, screens: &[Screen]) -> Result<usize, String> {
        for (o, s) in self.outputs.iter().zip(screens) {
            let mut b = self.buffer(o)?;
            b.put(s);
            self.set(o, &b)?;
        }
        Ok(self.outputs.len().min(screens.len()))
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
