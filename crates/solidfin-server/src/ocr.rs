//! Reading text in video frames with the OCR engine built into Windows 10/11
//! (`Windows.Media.Ocr`), using the languages installed for the Windows user.

/// Errors from the OCR engine as a whole (a single unreadable frame is not an error).
#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    /// No OCR engine for any of the user's languages, or not running on Windows.
    #[error("text recognition isn't available on this PC")]
    Unavailable,
}

/// Recognises the lines of text in each JPEG image. Blocking: call from `spawn_blocking`.
#[cfg(windows)]
pub fn read_lines(images: &[&[u8]]) -> Result<Vec<Vec<String>>, OcrError> {
    let engine = windows_ocr::engine().map_err(|err| {
        tracing::warn!(error = %err, "could not start Windows OCR");
        OcrError::Unavailable
    })?;
    Ok(images
        .iter()
        .map(|jpeg| {
            windows_ocr::read(&engine, jpeg).unwrap_or_else(|err| {
                tracing::debug!(error = %err, "could not read text in a frame; skipping it");
                Vec::new()
            })
        })
        .collect())
}

/// Recognises the lines of text in each JPEG image. Only available on Windows.
#[cfg(not(windows))]
pub fn read_lines(_images: &[&[u8]]) -> Result<Vec<Vec<String>>, OcrError> {
    Err(OcrError::Unavailable)
}

#[cfg(windows)]
mod windows_ocr {
    use windows::core::Result;
    use windows::Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    pub fn engine() -> Result<OcrEngine> {
        OcrEngine::TryCreateFromUserProfileLanguages()
    }

    pub fn read(engine: &OcrEngine, jpeg: &[u8]) -> Result<Vec<String>> {
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(jpeg)?;
        writer.StoreAsync()?.join()?;
        writer.DetachStream()?;
        stream.Seek(0)?;
        let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
        let bitmap = decoder.GetSoftwareBitmapAsync()?.join()?;
        let bitmap = SoftwareBitmap::Convert(&bitmap, BitmapPixelFormat::Bgra8)?;
        let result = engine.RecognizeAsync(&bitmap)?.join()?;
        result
            .Lines()?
            .into_iter()
            .map(|line| Ok(line.Text()?.to_string_lossy()))
            .collect()
    }
}
