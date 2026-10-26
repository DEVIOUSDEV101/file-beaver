//! Windows Imaging Component: decodes HEIC/AVIF (and anything else the `image` crate can't)
//! through codecs installed on the PC, encodes HEIC, and detects which codecs exist.
//! HEIC needs Microsoft's HEIF + HEVC extensions; AVIF needs HEIF + AV1.

use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use image::{DynamicImage, RgbaImage};
use windows::core::{GUID, HSTRING};
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::StructuredStorage::{CreateStreamOnHGlobal, GetHGlobalFromStream};
use windows::Win32::System::Com::*;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

use crate::formats::Caps;

/// Call once per thread before using WIC / WinRT. Harmless if already initialised.
pub fn init_thread() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn factory() -> Result<IWICImagingFactory> {
    init_thread();
    unsafe { Ok(CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?) }
}

pub fn decode(path: &std::path::Path) -> Result<DynamicImage> {
    unsafe {
        let f = factory()?;
        let decoder = f
            .CreateDecoderFromFilename(&HSTRING::from(path.as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .map_err(|e| anyhow!("Windows has no codec for this file ({})", e.message()))?;
        let frame = decoder.GetFrame(0)?;
        let conv = f.CreateFormatConverter()?;
        conv.Initialize(&frame, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let (mut w, mut h) = (0u32, 0u32);
        conv.GetSize(&mut w, &mut h)?;
        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        conv.CopyPixels(std::ptr::null(), w * 4, &mut buf)
            .map_err(|e| anyhow!("couldn't decode the image ({})", e.message()))?;
        Ok(DynamicImage::ImageRgba8(RgbaImage::from_raw(w, h, buf).context("bad image size")?))
    }
}

pub fn encode_heic(img: &DynamicImage) -> Result<Vec<u8>> {
    unsafe {
        let f = factory()?;
        let stream: IStream = CreateStreamOnHGlobal(None, true)?;
        let enc = f
            .CreateEncoder(&GUID_ContainerFormatHeif, std::ptr::null())
            .map_err(|_| anyhow!("HEIC export needs Microsoft's HEIF Image Extensions"))?;
        enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        let mut props = None;
        enc.CreateNewFrame(&mut frame, &mut props)?;
        let frame = frame.context("no frame")?;
        frame.Initialize(props.as_ref())?;
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        frame.SetSize(w, h)?;
        let mut fmt: GUID = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut fmt)?;
        let bgra: Vec<u8> = rgba.pixels().flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        let stride = w * 4;
        if fmt == GUID_WICPixelFormat32bppBGRA || fmt == GUID_WICPixelFormat32bppBGR {
            frame.WritePixels(h, stride, &bgra)?;
        } else {
            // Encoder asked for another layout: let WIC convert from a BGRA bitmap.
            let bmp = f.CreateBitmapFromMemory(w, h, &GUID_WICPixelFormat32bppBGRA, stride, &bgra)?;
            frame.WriteSource(&bmp, std::ptr::null())?;
        }
        frame
            .Commit()
            .map_err(|_| anyhow!("HEIC export needs Microsoft's HEVC Video Extensions"))?;
        enc.Commit()?;
        let hg = GetHGlobalFromStream(&stream)?;
        let size = GlobalSize(hg);
        let p = GlobalLock(hg) as *const u8;
        let out = std::slice::from_raw_parts(p, size).to_vec();
        let _ = GlobalUnlock(hg);
        Ok(out)
    }
}

fn has_wic_codec(container: &GUID, encoder: bool) -> bool {
    let Ok(f) = factory() else { return false };
    unsafe {
        if encoder {
            f.CreateEncoder(container, std::ptr::null()).is_ok()
        } else {
            f.CreateDecoder(container, std::ptr::null()).is_ok()
        }
    }
}

fn has_mft(category: GUID, subtype: GUID, output: bool) -> bool {
    unsafe {
        let _ = MFStartup(MF_VERSION, MFSTARTUP_LITE);
        let info = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: subtype };
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        let (input, out) = if output { (None, Some(&info as *const _)) } else { (Some(&info as *const _), None) };
        let ok = MFTEnumEx(category, MFT_ENUM_FLAG_ALL, input, out, &mut list, &mut count).is_ok();
        if !list.is_null() {
            for i in 0..count as usize {
                std::ptr::drop_in_place(list.add(i));
            }
            CoTaskMemFree(Some(list as *const _));
        }
        ok && count > 0
    }
}

/// Probes once which HEIC/AVIF codecs this PC has, and whether Word is installed.
pub fn caps() -> Caps {
    static CAPS: OnceLock<Caps> = OnceLock::new();
    *CAPS.get_or_init(|| {
        let heif_read = has_wic_codec(&GUID_ContainerFormatHeif, false);
        let hevc_dec = has_mft(MFT_CATEGORY_VIDEO_DECODER, MFVideoFormat_HEVC, false);
        let av1_dec = has_mft(MFT_CATEGORY_VIDEO_DECODER, MFVideoFormat_AV1, false);
        // An HEVC encoder in the codec list isn't enough (WIC only uses Microsoft's HEVC
        // extension, not e.g. a GPU encoder), so try a real tiny encode.
        let heic_encode = has_wic_codec(&GUID_ContainerFormatHeif, true)
            && encode_heic(&DynamicImage::ImageRgba8(RgbaImage::new(64, 64))).is_ok();
        Caps {
            heic_decode: heif_read && hevc_dec,
            heic_encode,
            avif_decode: heif_read && av1_dec,
            // Registry only; never starts Word.
            word: crate::word::installed(),
        }
    })
}
