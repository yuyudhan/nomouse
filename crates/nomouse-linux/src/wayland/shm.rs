//! Overlay frames in shared memory: labels composed into one transparent ARGB8888 frame per
//! output, written into a memfd with plain `pwrite` (no `mmap`, so no unsafe code), handed to
//! the compositor as a `wl_buffer`.

use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use nomouse_core::{PlatformError, Rect};
use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::QueueHandle;
use wayland_client::protocol::{wl_buffer, wl_shm};

use super::state::State;
use crate::x11::render::Label;

fn shm_error(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::Backend(format!("shared memory: {error}"))
}

/// Copies `labels` (premultiplied BGRA, the byte order of `wl_shm` ARGB8888 on little-endian
/// machines) into `frame`, a `width`×`height` frame whose top-left is `area`'s origin. Pixels
/// outside the frame are dropped; the rest of the frame is cleared to transparent.
pub(crate) fn compose(frame: &mut Vec<u8>, width: u32, height: u32, area: Rect, labels: &[Label]) {
    let stride = width as usize * 4;
    frame.clear();
    frame.resize(stride * height as usize, 0);
    for label in labels {
        let label_stride = label.bounds.width as usize * 4;
        let left = i64::from(label.bounds.x) - i64::from(area.x);
        let top = i64::from(label.bounds.y) - i64::from(area.y);
        for row in 0..i64::from(label.bounds.height) {
            let y = top + row;
            if y < 0 || y >= i64::from(height) {
                continue;
            }
            let first = left.max(0);
            let last = (left + i64::from(label.bounds.width)).min(i64::from(width));
            if first >= last {
                continue;
            }
            let (Ok(row), Ok(y), Ok(first), Ok(last), Ok(skip)) = (
                usize::try_from(row),
                usize::try_from(y),
                usize::try_from(first),
                usize::try_from(last),
                usize::try_from(left.min(0).unsigned_abs()),
            ) else {
                continue;
            };
            let source_start = row * label_stride + skip * 4;
            let count = (last - first) * 4;
            let target_start = y * stride + first * 4;
            if let (Some(target), Some(source)) = (
                frame.get_mut(target_start..target_start + count),
                label.pixels.get(source_start..source_start + count),
            ) {
                target.copy_from_slice(source);
            }
        }
    }
}

/// Puts `frame` (`width`×`height` ARGB8888) into a new `wl_buffer`.
///
/// # Errors
/// When the memfd cannot be created or written.
pub(crate) fn buffer(
    shm: &wl_shm::WlShm,
    frame: &[u8],
    width: u32,
    height: u32,
    qh: &QueueHandle<State>,
) -> Result<wl_buffer::WlBuffer, PlatformError> {
    let fd = memfd_create("nomouse-overlay", MemfdFlags::CLOEXEC).map_err(shm_error)?;
    let file = File::from(fd);
    file.set_len(frame.len() as u64).map_err(shm_error)?;
    file.write_all_at(frame, 0).map_err(shm_error)?;
    let size = i32::try_from(frame.len()).map_err(shm_error)?;
    let (width, height) = (
        i32::try_from(width).map_err(shm_error)?,
        i32::try_from(height).map_err(shm_error)?,
    );
    let pool = shm.create_pool(file.as_fd(), size, qh, ());
    let buffer = pool.create_buffer(
        0,
        width,
        height,
        width * 4,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    // The buffer keeps the memory alive; the pool is no longer needed.
    pool.destroy();
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(x: i32, y: i32, width: u32, height: u32, byte: u8) -> Label {
        Label {
            bounds: Rect::new(x, y, width, height),
            pixels: vec![byte; (width * height * 4) as usize],
        }
    }

    fn pixel(frame: &[u8], width: u32, x: usize, y: usize) -> &[u8] {
        let at = (y * width as usize + x) * 4;
        &frame[at..at + 4]
    }

    #[test]
    fn labels_land_at_their_output_relative_position() {
        let mut frame = Vec::new();
        let area = Rect::new(100, 50, 10, 10);
        compose(&mut frame, 10, 10, area, &[label(102, 53, 2, 2, 7)]);
        assert_eq!(frame.len(), 400);
        assert_eq!(pixel(&frame, 10, 2, 3), &[7, 7, 7, 7]);
        assert_eq!(pixel(&frame, 10, 3, 4), &[7, 7, 7, 7]);
        assert_eq!(pixel(&frame, 10, 4, 3), &[0, 0, 0, 0]);
        assert_eq!(pixel(&frame, 10, 1, 3), &[0, 0, 0, 0]);
    }

    #[test]
    fn labels_are_clipped_to_the_frame_and_frames_are_cleared() {
        let mut frame = vec![9; 16];
        let area = Rect::new(0, 0, 4, 4);
        compose(
            &mut frame,
            4,
            4,
            area,
            &[label(-1, -1, 3, 3, 5), label(3, 3, 4, 4, 6)],
        );
        assert_eq!(pixel(&frame, 4, 0, 0), &[5, 5, 5, 5]);
        assert_eq!(pixel(&frame, 4, 1, 1), &[5, 5, 5, 5]);
        assert_eq!(pixel(&frame, 4, 2, 2), &[0, 0, 0, 0]);
        assert_eq!(pixel(&frame, 4, 3, 3), &[6, 6, 6, 6]);
        assert_eq!(pixel(&frame, 4, 3, 0), &[0, 0, 0, 0]);
    }
}
