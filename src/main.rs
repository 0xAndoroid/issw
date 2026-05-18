#[cfg(not(target_os = "macos"))]
compile_error!("issw only supports macOS.");

use std::env;
use std::ffi::{c_char, c_void, CStr, OsString};
use std::io::{self, Write as _};
use std::process::ExitCode;
use std::ptr::NonNull;

type Boolean = u8;
type CFIndex = isize;
type CFStringEncoding = u32;
type OSStatus = i32;

type CFArrayRef = *const c_void;
type CFBooleanRef = *const c_void;
type CFStringRef = *const c_void;
type CFTypeRef = *const c_void;
type TISInputSourceRef = *const c_void;

const K_CF_STRING_ENCODING_UTF8: CFStringEncoding = 0x0800_0100;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    static kTISCategoryKeyboardInputSource: CFStringRef;
    static kTISPropertyInputSourceCategory: CFStringRef;
    static kTISPropertyInputSourceID: CFStringRef;
    static kTISPropertyInputSourceIsSelectCapable: CFStringRef;
    static kTISPropertyLocalizedName: CFStringRef;

    fn TISCopyCurrentKeyboardInputSource() -> TISInputSourceRef;
    fn TISCreateInputSourceList(properties: CFTypeRef, includeAllInstalled: Boolean) -> CFArrayRef;
    fn TISGetInputSourceProperty(
        inputSource: TISInputSourceRef,
        propertyKey: CFStringRef,
    ) -> CFTypeRef;
    fn TISSelectInputSource(inputSource: TISInputSourceRef) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(theArray: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(theArray: CFArrayRef, idx: CFIndex) -> *const c_void;
    fn CFBooleanGetValue(boolean: CFBooleanRef) -> Boolean;
    fn CFEqual(cf1: CFTypeRef, cf2: CFTypeRef) -> Boolean;
    fn CFRelease(cf: CFTypeRef);
    fn CFStringGetCString(
        theString: CFStringRef,
        buffer: *mut c_char,
        bufferSize: CFIndex,
        encoding: CFStringEncoding,
    ) -> Boolean;
    fn CFStringGetCStringPtr(theString: CFStringRef, encoding: CFStringEncoding) -> *const c_char;
    fn CFStringGetLength(theString: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: CFStringEncoding) -> CFIndex;
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "issw: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Error> {
    match Command::parse(env::args_os().skip(1))? {
        Command::Help => {
            print_usage()?;
            Ok(())
        }
        Command::List => {
            let list = SourceList::load()?;
            let mut stdout = io::stdout().lock();

            for source in list.sources() {
                writeln!(stdout, "{}\t{}", source.info.id, source.info.name)?;
            }

            Ok(())
        }
        Command::Current => {
            let source = current_source()?;
            writeln!(io::stdout().lock(), "{}\t{}", source.id, source.name)?;
            Ok(())
        }
        Command::Switch(query) => switch_source(&query),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Help,
    List,
    Current,
    Switch(String),
}

impl Command {
    fn parse(args: impl Iterator<Item = OsString>) -> Result<Self, Error> {
        let args = args
            .map(|arg| {
                arg.into_string()
                    .map_err(|_| Error::InvalidArgument("arguments must be valid UTF-8".to_owned()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let Some(command) = args.first() else {
            return Ok(Self::Help);
        };

        match command.as_str() {
            "-h" | "--help" => {
                reject_extra_args(&args)?;
                Ok(Self::Help)
            }
            "list" => {
                reject_extra_args(&args)?;
                Ok(Self::List)
            }
            "current" => {
                reject_extra_args(&args)?;
                Ok(Self::Current)
            }
            command if command.starts_with('-') => {
                Err(Error::InvalidArgument(format!("unknown option: {command}")))
            }
            _ => Ok(Self::Switch(args.join(" "))),
        }
    }
}

fn reject_extra_args(args: &[String]) -> Result<(), Error> {
    if args.len() == 1 {
        Ok(())
    } else {
        Err(Error::InvalidArgument(format!(
            "`{}` does not accept extra arguments",
            args[0]
        )))
    }
}

struct SourceList {
    sources: Vec<InputSource>,
    _array: OwnedCf,
}

impl SourceList {
    fn load() -> Result<Self, Error> {
        let array = unsafe {
            OwnedCf::from_create(
                TISCreateInputSourceList(std::ptr::null(), 0) as CFTypeRef,
                "TISCreateInputSourceList",
            )?
        };

        let count = unsafe { CFArrayGetCount(array.as_array()) };
        if count < 0 {
            return Err(Error::System("input source count was negative"));
        }

        let mut sources = Vec::with_capacity(count as usize);
        for index in 0..count {
            let raw = unsafe { CFArrayGetValueAtIndex(array.as_array(), index) };
            let Some(raw) = NonNull::new(raw.cast_mut()) else {
                continue;
            };

            let source_ref = raw.as_ptr().cast_const();
            if !is_selectable_keyboard_source(source_ref) {
                continue;
            }

            let Some(source) = input_source_from_ref(raw) else {
                continue;
            };

            sources.push(source);
        }

        Ok(Self {
            sources,
            _array: array,
        })
    }

    fn sources(&self) -> &[InputSource] {
        &self.sources
    }
}

#[derive(Clone)]
struct InputSource {
    raw: NonNull<c_void>,
    info: SourceInfo,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceInfo {
    id: String,
    name: String,
}

fn current_source() -> Result<SourceInfo, Error> {
    let source = unsafe {
        OwnedCf::from_create(
            TISCopyCurrentKeyboardInputSource() as CFTypeRef,
            "TISCopyCurrentKeyboardInputSource",
        )?
    };

    input_source_info(source.non_null()).ok_or(Error::MissingProperty("current input source"))
}

fn switch_source(query: &str) -> Result<(), Error> {
    let list = SourceList::load()?;
    let source = find_source(list.sources(), query)?;
    let status = unsafe { TISSelectInputSource(source.raw.as_ptr().cast_const()) };

    if status == 0 {
        Ok(())
    } else {
        Err(Error::OsStatus("TISSelectInputSource", status))
    }
}

fn find_source<'a>(sources: &'a [InputSource], query: &str) -> Result<&'a InputSource, Error> {
    if let Some(source) = sources
        .iter()
        .find(|source| source.info.id == query || source.info.name == query)
    {
        return Ok(source);
    }

    let query = query.to_lowercase();
    let mut matches = sources.iter().filter(|source| {
        source.info.id.to_lowercase().contains(&query)
            || source.info.name.to_lowercase().contains(&query)
    });

    let Some(first) = matches.next() else {
        return Err(Error::NoMatch);
    };

    if matches.next().is_some() {
        return Err(Error::AmbiguousMatch);
    }

    Ok(first)
}

fn input_source_from_ref(raw: NonNull<c_void>) -> Option<InputSource> {
    let info = input_source_info(raw)?;

    Some(InputSource { raw, info })
}

fn input_source_info(raw: NonNull<c_void>) -> Option<SourceInfo> {
    let source = raw.as_ptr().cast_const();
    let id = string_property(source, unsafe { kTISPropertyInputSourceID })?;
    let name = string_property(source, unsafe { kTISPropertyLocalizedName })?;

    Some(SourceInfo { id, name })
}

fn is_selectable_keyboard_source(source: TISInputSourceRef) -> bool {
    if !bool_property(source, unsafe { kTISPropertyInputSourceIsSelectCapable }) {
        return false;
    }

    let category = unsafe { TISGetInputSourceProperty(source, kTISPropertyInputSourceCategory) };
    !category.is_null() && unsafe { CFEqual(category, kTISCategoryKeyboardInputSource) != 0 }
}

fn string_property(source: TISInputSourceRef, key: CFStringRef) -> Option<String> {
    let value = unsafe { TISGetInputSourceProperty(source, key) };
    cf_string_to_string(value as CFStringRef)
}

fn bool_property(source: TISInputSourceRef, key: CFStringRef) -> bool {
    let value = unsafe { TISGetInputSourceProperty(source, key) };
    !value.is_null() && unsafe { CFBooleanGetValue(value as CFBooleanRef) != 0 }
}

fn cf_string_to_string(value: CFStringRef) -> Option<String> {
    if value.is_null() {
        return None;
    }

    let direct = unsafe { CFStringGetCStringPtr(value, K_CF_STRING_ENCODING_UTF8) };
    if !direct.is_null() {
        return unsafe { CStr::from_ptr(direct) }
            .to_str()
            .ok()
            .map(str::to_owned);
    }

    let length = unsafe { CFStringGetLength(value) };
    if length < 0 {
        return None;
    }

    let max_size = unsafe { CFStringGetMaximumSizeForEncoding(length, K_CF_STRING_ENCODING_UTF8) };
    if max_size < 0 {
        return None;
    }

    let buffer_size = max_size.checked_add(1)?;
    let mut buffer = vec![0; usize::try_from(buffer_size).ok()?];

    let copied = unsafe {
        CFStringGetCString(
            value,
            buffer.as_mut_ptr(),
            buffer_size,
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    if copied == 0 {
        return None;
    }

    unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

struct OwnedCf {
    ptr: NonNull<c_void>,
}

impl OwnedCf {
    unsafe fn from_create(ptr: CFTypeRef, operation: &'static str) -> Result<Self, Error> {
        NonNull::new(ptr.cast_mut())
            .map(|ptr| Self { ptr })
            .ok_or(Error::NullResult(operation))
    }

    fn as_array(&self) -> CFArrayRef {
        self.ptr.as_ptr().cast_const()
    }

    fn non_null(&self) -> NonNull<c_void> {
        self.ptr
    }
}

impl Drop for OwnedCf {
    fn drop(&mut self) {
        unsafe {
            CFRelease(self.ptr.as_ptr().cast_const());
        }
    }
}

#[derive(Debug)]
enum Error {
    AmbiguousMatch,
    InvalidArgument(String),
    MissingProperty(&'static str),
    NoMatch,
    NullResult(&'static str),
    OsStatus(&'static str, OSStatus),
    System(&'static str),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AmbiguousMatch => write!(
                formatter,
                "multiple input sources matched; use `issw list` and pass an exact id or name"
            ),
            Self::InvalidArgument(message) => formatter.write_str(message),
            Self::MissingProperty(source) => write!(formatter, "{source} is missing id or name"),
            Self::NoMatch => write!(
                formatter,
                "no input source matched; use `issw list` to see available sources"
            ),
            Self::NullResult(operation) => write!(formatter, "{operation} returned null"),
            Self::OsStatus(operation, status) => write!(formatter, "{operation} failed: {status}"),
            Self::System(message) => formatter.write_str(message),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn print_usage() -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "usage:")?;
    writeln!(stderr, "  issw list")?;
    writeln!(stderr, "  issw current")?;
    writeln!(stderr, "  issw <id-or-name>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_args_as_help() -> Result<(), Error> {
        assert_eq!(Command::parse(args(&[]))?, Command::Help);
        Ok(())
    }

    #[test]
    fn parses_multi_word_switch_query() -> Result<(), Error> {
        assert_eq!(
            Command::parse(args(&["ABC", "-", "Extended"]))?,
            Command::Switch("ABC - Extended".to_owned())
        );
        Ok(())
    }

    #[test]
    fn rejects_extra_args_for_builtin_commands() {
        assert!(matches!(
            Command::parse(args(&["list", "extra"])),
            Err(Error::InvalidArgument(_))
        ));
    }

    #[test]
    fn exact_match_wins_before_substring_matching() -> Result<(), Error> {
        let sources = [
            source("abc", "ABC"),
            source("abc-extended", "ABC - Extended"),
        ];

        let matched = find_source(&sources, "ABC")?;

        assert_eq!(matched.info.id, "abc");
        Ok(())
    }

    #[test]
    fn substring_matching_requires_unique_match() -> Result<(), Error> {
        let sources = [
            source("us", "U.S."),
            source("us-international", "U.S. International"),
        ];

        let matched = find_source(&sources, "U.S.")?;
        assert_eq!(matched.info.id, "us");
        assert!(matches!(
            find_source(&sources, "u.s"),
            Err(Error::AmbiguousMatch)
        ));
        Ok(())
    }

    fn args(values: &[&str]) -> impl Iterator<Item = OsString> {
        values
            .iter()
            .map(|value| OsString::from(*value))
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn source(id: &str, name: &str) -> InputSource {
        InputSource {
            raw: NonNull::dangling(),
            info: SourceInfo {
                id: id.to_owned(),
                name: name.to_owned(),
            },
        }
    }
}
