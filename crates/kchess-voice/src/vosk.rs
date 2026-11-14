//! The part of the Vosk C API (`vosk_api.h`) the helper uses, loaded from the shared library at
//! run time. Models and recognizers borrow the loaded library, so neither outlives it.
// The one module that calls C; every unsafe block carries its SAFETY comment.
#![allow(unsafe_code)]

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
    functions: Functions,
    // Last: the function pointers stay valid only while the library is loaded. `None` for the
    // tests' fakes, which live in this executable.
    _library: Option<Library>,
}

/// The functions the helper calls.
struct Functions {
    model_new: ModelNew,
    model_free: Free,
    recognizer_new_grm: RecognizerNewGrm,
    recognizer_set_words: SetWords,
    recognizer_accept_waveform_s: AcceptWaveformS,
    recognizer_result: ResultText,
    recognizer_partial_result: ResultText,
    recognizer_final_result: ResultText,
    recognizer_free: Free,
}

/// Where the functions come from: the loaded library, or fakes in the tests.
trait Symbols {
    /// Copies the function `name`, typed as `T`.
    ///
    /// # Safety
    /// `T` must be the function's real signature from `vosk_api.h`.
    unsafe fn symbol<T: Copy>(&self, name: &[u8]) -> Result<T, String>;
}

impl Symbols for Library {
    unsafe fn symbol<T: Copy>(&self, name: &[u8]) -> Result<T, String> {
        // SAFETY: the caller guarantees `T` matches the C declaration; the copied pointer is only
        // used while the library (kept in `Vosk`) stays loaded.
        unsafe { self.get::<T>(name) }
            .map(|symbol| *symbol)
            .map_err(|cause| format!("libvosk lacks {}: {cause}", String::from_utf8_lossy(name)))
    }
}

impl Functions {
    /// Takes the functions from `symbols` and silences Kaldi's logging. The caller keeps the
    /// library that holds them loaded for as long as the answer lives.
    ///
    /// # Safety
    /// `symbols` must serve each name with the signature `vosk_api.h` declares for it.
    unsafe fn bind(symbols: &impl Symbols) -> Result<Functions, String> {
        // SAFETY: each type below is copied from the matching declaration in vosk_api.h, which
        // the caller guarantees `symbols` serves.
        unsafe {
            let set_log_level: SetLogLevel = symbols.symbol(b"vosk_set_log_level\0")?;
            set_log_level(-1);
            Ok(Functions {
                model_new: symbols.symbol(b"vosk_model_new\0")?,
                model_free: symbols.symbol(b"vosk_model_free\0")?,
                recognizer_new_grm: symbols.symbol(b"vosk_recognizer_new_grm\0")?,
                recognizer_set_words: symbols.symbol(b"vosk_recognizer_set_words\0")?,
                recognizer_accept_waveform_s: symbols
                    .symbol(b"vosk_recognizer_accept_waveform_s\0")?,
                recognizer_result: symbols.symbol(b"vosk_recognizer_result\0")?,
                recognizer_partial_result: symbols.symbol(b"vosk_recognizer_partial_result\0")?,
                recognizer_final_result: symbols.symbol(b"vosk_recognizer_final_result\0")?,
                recognizer_free: symbols.symbol(b"vosk_recognizer_free\0")?,
            })
        }
    }
}

impl Vosk {
    /// Loads libvosk from `path` and silences Kaldi's logging (stdout belongs to the protocol).
    pub fn load(path: &Path) -> Result<Vosk, String> {
        // SAFETY: loading runs the library's initializers; libvosk is the pinned upstream build
        // shipped beside this executable, with no initializers beyond its C++ runtime's.
        let library = unsafe { Library::new(path) }
            .map_err(|cause| format!("Could not load {}: {cause}", path.display()))?;
        // SAFETY: `library` is libvosk, whose functions `bind` types from vosk_api.h.
        let functions = unsafe { Functions::bind(&library) }?;
        Ok(Vosk {
            functions,
            _library: Some(library),
        })
    }

    pub fn model(&self, path: &Path) -> Result<Model<'_>, String> {
        let text = path
            .to_str()
            .and_then(|text| CString::new(text).ok())
            .ok_or("The voice model path is not valid text.")?;
        // SAFETY: `text` is a valid NUL-terminated string for the duration of the call.
        let pointer = unsafe { (self.functions.model_new)(text.as_ptr()) };
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
        unsafe { (self.vosk.functions.model_free)(self.pointer.as_ptr()) }
    }
}

impl Model<'_> {
    /// A recognizer limited to `grammar` (a JSON array of phrases), reporting each word.
    pub fn recognizer(&self, sample_rate: f32, grammar: &str) -> Result<Recognizer<'_>, String> {
        let grammar = CString::new(grammar).map_err(|_| "The grammar holds a NUL character.")?;
        // SAFETY: the model pointer is live (borrowed) and `grammar` is NUL-terminated.
        let pointer = unsafe {
            (self.vosk.functions.recognizer_new_grm)(
                self.pointer.as_ptr(),
                sample_rate,
                grammar.as_ptr(),
            )
        };
        let pointer = NonNull::new(pointer).ok_or("Speech recognition couldn’t start.")?;
        // SAFETY: `pointer` is the recognizer just created.
        unsafe { (self.vosk.functions.recognizer_set_words)(pointer.as_ptr(), 1) };
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
        unsafe { (self.vosk.functions.recognizer_free)(self.pointer.as_ptr()) }
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
            (self.vosk.functions.recognizer_accept_waveform_s)(
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
        unsafe {
            owned((self.vosk.functions.recognizer_result)(
                self.pointer.as_ptr(),
            ))
        }
    }

    pub fn partial(&mut self) -> String {
        // SAFETY: as `result`.
        unsafe {
            owned((self.vosk.functions.recognizer_partial_result)(
                self.pointer.as_ptr(),
            ))
        }
    }

    pub fn finish(&mut self) -> String {
        // SAFETY: as `result`.
        unsafe {
            owned((self.vosk.functions.recognizer_final_result)(
                self.pointer.as_ptr(),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    thread_local! {
        static LOG_LEVEL: Cell<Option<c_int>> = const { Cell::new(None) };
        static FREED: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
        static WORDS: Cell<Option<c_int>> = const { Cell::new(None) };
        static NO_MODEL: Cell<bool> = const { Cell::new(false) };
    }

    /// Stands in for the opaque model and recognizer handles.
    static HANDLE: u8 = 0;

    fn handle() -> *mut c_void {
        (&raw const HANDLE).cast_mut().cast()
    }

    extern "C" fn set_log_level(level: c_int) {
        LOG_LEVEL.set(Some(level));
    }
    extern "C" fn model_new(_path: *const c_char) -> *mut c_void {
        if NO_MODEL.get() {
            std::ptr::null_mut()
        } else {
            handle()
        }
    }
    extern "C" fn model_free(_model: *mut c_void) {
        FREED.with_borrow_mut(|freed| freed.push("model"));
    }
    extern "C" fn recognizer_new_grm(
        _model: *mut c_void,
        _rate: c_float,
        _grammar: *const c_char,
    ) -> *mut c_void {
        handle()
    }
    extern "C" fn recognizer_set_words(_recognizer: *mut c_void, words: c_int) {
        WORDS.set(Some(words));
    }
    /// Answers the first sample, as Vosk answers 0 (still listening), 1 (an utterance ended)
    /// or -1 (failed).
    extern "C" fn accept_waveform_s(
        _recognizer: *mut c_void,
        samples: *const c_short,
        length: c_int,
    ) -> c_int {
        if length == 0 {
            return 0;
        }
        // SAFETY: `Recognizer::accept` passes `length` valid samples.
        c_int::from(unsafe { *samples })
    }
    extern "C" fn result(_recognizer: *mut c_void) -> *const c_char {
        c"{\"text\" : \"e four\"}".as_ptr()
    }
    extern "C" fn partial_result(_recognizer: *mut c_void) -> *const c_char {
        c"{\"partial\" : \"e\"}".as_ptr()
    }
    extern "C" fn final_result(_recognizer: *mut c_void) -> *const c_char {
        c"{\"text\" : \"\"}".as_ptr()
    }
    extern "C" fn recognizer_free(_recognizer: *mut c_void) {
        FREED.with_borrow_mut(|freed| freed.push("recognizer"));
    }

    /// The fakes above by their libvosk names; `missing` is left out.
    struct Fakes {
        missing: &'static [u8],
    }

    impl Symbols for Fakes {
        unsafe fn symbol<T: Copy>(&self, name: &[u8]) -> Result<T, String> {
            let address = match name {
                _ if name == self.missing => None,
                b"vosk_set_log_level\0" => Some(set_log_level as SetLogLevel as usize),
                b"vosk_model_new\0" => Some(model_new as ModelNew as usize),
                b"vosk_model_free\0" => Some(model_free as Free as usize),
                b"vosk_recognizer_new_grm\0" => {
                    Some(recognizer_new_grm as RecognizerNewGrm as usize)
                }
                b"vosk_recognizer_set_words\0" => Some(recognizer_set_words as SetWords as usize),
                b"vosk_recognizer_accept_waveform_s\0" => {
                    Some(accept_waveform_s as AcceptWaveformS as usize)
                }
                b"vosk_recognizer_result\0" => Some(result as ResultText as usize),
                b"vosk_recognizer_partial_result\0" => Some(partial_result as ResultText as usize),
                b"vosk_recognizer_final_result\0" => Some(final_result as ResultText as usize),
                b"vosk_recognizer_free\0" => Some(recognizer_free as Free as usize),
                _ => None,
            };
            let address = address.ok_or_else(|| "missing".to_owned())?;
            // SAFETY: the caller asks for each name with its vosk_api.h signature, which is the
            // signature of the fake served for it.
            Ok(unsafe { std::mem::transmute_copy::<usize, T>(&address) })
        }
    }

    fn fake() -> Vosk {
        Vosk {
            // SAFETY: `Fakes` serves each name with its vosk_api.h signature.
            functions: unsafe { Functions::bind(&Fakes { missing: b"" }) }.expect("bound"),
            _library: None,
        }
    }

    #[test]
    fn binding_silences_kaldis_logging() {
        let _vosk = fake();
        assert_eq!(LOG_LEVEL.get(), Some(-1));
    }

    #[test]
    fn a_missing_function_fails_the_binding() {
        // SAFETY: as `fake`.
        let error = unsafe {
            Functions::bind(&Fakes {
                missing: b"vosk_model_free\0",
            })
        }
        .err()
        .expect("refused");
        assert_eq!(error, "missing");
    }

    #[test]
    fn a_model_that_does_not_load_is_an_error() {
        let vosk = fake();
        NO_MODEL.set(true);
        assert!(vosk.model(Path::new("model")).is_err());
        NO_MODEL.set(false);
    }

    #[test]
    fn a_recognizer_reports_words_and_answers_each_call() {
        let vosk = fake();
        let model = vosk.model(Path::new("model")).expect("model");
        let mut recognizer = model
            .recognizer(16_000.0, "[\"e four\"]")
            .expect("recognizer");
        assert_eq!(WORDS.get(), Some(1));

        assert_eq!(recognizer.accept(&[0, 5]), Ok(false));
        assert_eq!(recognizer.accept(&[1]), Ok(true));
        assert!(recognizer.accept(&[-1]).is_err());
        assert!(recognizer.accept(&[2]).is_err());
        assert_eq!(recognizer.result(), "{\"text\" : \"e four\"}");
        assert_eq!(recognizer.partial(), "{\"partial\" : \"e\"}");
        assert_eq!(recognizer.finish(), "{\"text\" : \"\"}");
        assert!(model.recognizer(16_000.0, "a\0b").is_err());
    }

    #[test]
    fn recognizers_then_models_are_freed_once() {
        let vosk = fake();
        let model = vosk.model(Path::new("model")).expect("model");
        let recognizer = model.recognizer(16_000.0, "[]").expect("recognizer");
        drop(recognizer);
        drop(model);
        assert_eq!(FREED.take(), ["recognizer", "model"]);
    }

    #[test]
    fn a_null_result_reads_as_empty() {
        // SAFETY: null is allowed.
        assert_eq!(unsafe { owned(std::ptr::null()) }, "");
    }
}
