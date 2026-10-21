//! The part of the Vosk C API (`vosk_api.h`) the helper uses, loaded from the shared library at
//! run time. Models and recognizers borrow the loaded library, so neither outlives it.

use std::ffi::{CStr, CString, c_char, c_float, c_int, c_short, c_void};
use std::path::Path;
use std::ptr::NonNull;

use libloading::Library;

type ModelNew = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type Free = unsafe extern "C" fn(*mut c_void);
type RecognizerNewGrm = unsafe extern "C" fn(*mut c_void, c_float, *const c_char) -> *mut c_void;
type SetWords = unsafe extern "C" fn(*mut c_void, c_int);
type AcceptWaveformS = unsafe extern "C" fn(*mut c_void, *const c_short, c_int) -> c_int;
type ResultText = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type SetLogLevel = unsafe extern "C" fn(c_int);

/// The loaded library and the functions taken from it.
pub struct Vosk {
    model_new: ModelNew,
    model_free: Free,
    recognizer_new_grm: RecognizerNewGrm,
    recognizer_set_words: SetWords,
    recognizer_accept_waveform_s: AcceptWaveformS,
    recognizer_result: ResultText,
    recognizer_partial_result: ResultText,
    recognizer_final_result: ResultText,
    recognizer_free: Free,
    // Last: the function pointers above stay valid only while the library is loaded.
    _library: Library,
}

/// Copies the function `name` out of `library`, typed as `T`.
///
/// # Safety
/// `T` must be the function's real signature from `vosk_api.h`.
unsafe fn symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
    // SAFETY: the caller guarantees `T` matches the C declaration; the copied pointer is only
    // used while `library` (kept in `Vosk`) stays loaded.
    unsafe { library.get::<T>(name) }
        .map(|symbol| *symbol)
        .map_err(|cause| format!("libvosk lacks {}: {cause}", String::from_utf8_lossy(name)))
}

impl Vosk {
    /// Loads libvosk from `path` and silences Kaldi's logging (stdout belongs to the protocol).
    pub fn load(path: &Path) -> Result<Vosk, String> {
        // SAFETY: loading runs the library's initializers; libvosk is the pinned upstream build
        // shipped beside this executable, with no initializers beyond its C++ runtime's.
        let library = unsafe { Library::new(path) }
            .map_err(|cause| format!("Could not load {}: {cause}", path.display()))?;
        // SAFETY: each type below is copied from the matching declaration in vosk_api.h.
        let vosk = unsafe {
            let set_log_level: SetLogLevel = symbol(&library, b"vosk_set_log_level\0")?;
            set_log_level(-1);
            Vosk {
                model_new: symbol(&library, b"vosk_model_new\0")?,
                model_free: symbol(&library, b"vosk_model_free\0")?,
                recognizer_new_grm: symbol(&library, b"vosk_recognizer_new_grm\0")?,
                recognizer_set_words: symbol(&library, b"vosk_recognizer_set_words\0")?,
                recognizer_accept_waveform_s: symbol(
                    &library,
                    b"vosk_recognizer_accept_waveform_s\0",
                )?,
                recognizer_result: symbol(&library, b"vosk_recognizer_result\0")?,
                recognizer_partial_result: symbol(&library, b"vosk_recognizer_partial_result\0")?,
                recognizer_final_result: symbol(&library, b"vosk_recognizer_final_result\0")?,
                recognizer_free: symbol(&library, b"vosk_recognizer_free\0")?,
                _library: library,
            }
        };
        Ok(vosk)
    }

    /// Loads the model folder at `path`.
    pub fn model(&self, path: &Path) -> Result<Model<'_>, String> {
        let text = path
            .to_str()
            .and_then(|text| CString::new(text).ok())
            .ok_or("The voice model path is not valid text.")?;
        // SAFETY: `text` is a valid NUL-terminated string for the duration of the call.
        let pointer = unsafe { (self.model_new)(text.as_ptr()) };
        NonNull::new(pointer)
            .map(|pointer| Model {
                vosk: self,
                pointer,
            })
            .ok_or_else(|| "The offline voice model couldn’t load.".to_owned())
    }
}

pub struct Model<'a> {
    vosk: &'a Vosk,
    pointer: NonNull<c_void>,
}

impl Drop for Model<'_> {
    fn drop(&mut self) {
        // SAFETY: `pointer` came from `vosk_model_new` and is freed once; recognizers borrow
        // the model, so none is alive here.
        unsafe { (self.vosk.model_free)(self.pointer.as_ptr()) }
    }
}

impl Model<'_> {
    /// A recognizer limited to `grammar` (a JSON array of phrases), reporting each word.
    pub fn recognizer(&self, sample_rate: f32, grammar: &str) -> Result<Recognizer<'_>, String> {
        let grammar = CString::new(grammar).map_err(|_| "The grammar holds a NUL character.")?;
        // SAFETY: the model pointer is live (borrowed) and `grammar` is NUL-terminated.
        let pointer = unsafe {
            (self.vosk.recognizer_new_grm)(self.pointer.as_ptr(), sample_rate, grammar.as_ptr())
        };
        let pointer = NonNull::new(pointer).ok_or("Speech recognition couldn’t start.")?;
        // SAFETY: `pointer` is the recognizer just created.
        unsafe { (self.vosk.recognizer_set_words)(pointer.as_ptr(), 1) };
        Ok(Recognizer {
            vosk: self.vosk,
            pointer,
            _model: std::marker::PhantomData,
        })
    }
}

pub struct Recognizer<'a> {
    vosk: &'a Vosk,
    pointer: NonNull<c_void>,
    _model: std::marker::PhantomData<&'a Model<'a>>,
}

impl Drop for Recognizer<'_> {
    fn drop(&mut self) {
        // SAFETY: `pointer` came from `vosk_recognizer_new_grm` and is freed once.
        unsafe { (self.vosk.recognizer_free)(self.pointer.as_ptr()) }
    }
}

/// Copies a result string the recognizer owns (valid until its next call).
///
/// # Safety
/// `text` must be null or a NUL-terminated string that stays valid during this call.
unsafe fn owned(text: *const c_char) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: guaranteed by the caller.
    unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned()
}

impl Recognizer<'_> {
    /// Feeds samples; `Ok(true)` when an utterance ended and `result` holds it.
    pub fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
        let length = c_int::try_from(samples.len()).map_err(|_| "Audio chunk too large.")?;
        // SAFETY: `samples` is valid for `length` shorts; the recognizer is live.
        match unsafe {
            (self.vosk.recognizer_accept_waveform_s)(
                self.pointer.as_ptr(),
                samples.as_ptr(),
                length,
            )
        } {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("Speech recognition stopped. Try again.".to_owned()),
        }
    }

    pub fn result(&mut self) -> String {
        // SAFETY: the recognizer is live; the string is copied before its next call.
        unsafe { owned((self.vosk.recognizer_result)(self.pointer.as_ptr())) }
    }

    pub fn partial(&mut self) -> String {
        // SAFETY: as `result`.
        unsafe { owned((self.vosk.recognizer_partial_result)(self.pointer.as_ptr())) }
    }

    pub fn finish(&mut self) -> String {
        // SAFETY: as `result`.
        unsafe { owned((self.vosk.recognizer_final_result)(self.pointer.as_ptr())) }
    }
}
