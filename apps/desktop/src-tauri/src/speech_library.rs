use libloading::Library;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::path::Path;

pub(super) struct SpeechLibrary(ManuallyDrop<Library>);

impl SpeechLibrary {
    pub(super) unsafe fn load(path: &Path) -> Result<Self, libloading::Error> {
        #[cfg(windows)]
        let library =
            libloading::os::windows::Library::load_with_flags(path, 0x100 | 0x800)?.into();
        #[cfg(not(windows))]
        let library = Library::new(path)?;
        Ok(Self(ManuallyDrop::new(library)))
    }
}

impl Deref for SpeechLibrary {
    type Target = Library;
    fn deref(&self) -> &Library {
        &self.0
    }
}

impl Drop for SpeechLibrary {
    fn drop(&mut self) {
        // Windows OpenMP threads can outlive model destruction. Keep the DLL
        // loaded until isolated process exit, rather than unloading their code.
        #[cfg(not(windows))]
        unsafe {
            ManuallyDrop::drop(&mut self.0);
        }
    }
}
