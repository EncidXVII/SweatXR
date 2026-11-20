// Copyright (C) 2026 EncidXVII
// SPDX-License-Identifier: GPL-3.0-only

//! Local UTC offset from the C runtime. std has no time zone support and no tz crate is in the
//! allowed set; both CRTs already link into every binary.

use std::os::raw::c_int;

#[repr(C)]
struct Tm {
    fields: [c_int; 9],
    // glibc appends tm_gmtoff and tm_zone; reserve room so the CRT never writes past us.
    _tail: [u64; 4],
}

#[cfg(windows)]
unsafe extern "C" {
    fn _localtime64_s(tm: *mut Tm, t: *const i64) -> c_int;
    fn _mkgmtime64(tm: *mut Tm) -> i64;
}

#[cfg(unix)]
unsafe extern "C" {
    fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm;
    fn timegm(tm: *mut Tm) -> i64;
}

/// Seconds to add to UTC to get local wall time at `ts`, DST included.
pub fn local_offset(ts: i64) -> Option<i32> {
    let mut tm = Tm {
        fields: [0; 9],
        _tail: [0; 4],
    };
    #[cfg(windows)]
    // SAFETY: both pointers are valid for the call; the CRT fills `tm` and reads only `ts`.
    let local = unsafe {
        if _localtime64_s(&mut tm, &ts) != 0 {
            return None;
        }
        _mkgmtime64(&mut tm)
    };
    #[cfg(unix)]
    // SAFETY: as above; `localtime_r` is the reentrant variant, so no shared static is touched.
    let local = unsafe {
        if localtime_r(&ts, &mut tm).is_null() {
            return None;
        }
        timegm(&mut tm)
    };
    i32::try_from(local - ts).ok()
}
