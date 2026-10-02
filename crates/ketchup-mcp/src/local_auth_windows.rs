//! Atomic Windows creation with a protected current-user-only DACL.
use super::*;
use std::{
    ffi::c_void,
    os::windows::{
        ffi::OsStrExt,
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle},
    },
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{Authorization::*, *},
    Storage::FileSystem::CreateFileW,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
struct Token(HANDLE);
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Descriptor(*mut c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn user() -> io::Result<Vec<usize>> {
    // SAFETY: buffers are pointer-aligned and live through the Win32 calls.
    unsafe {
        let mut raw = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Token(raw);
        let mut size = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
        let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(buffer)
    }
}
pub fn create(path: &Path) -> io::Result<File> {
    let user = user()?;
    // SAFETY: aligned descriptor/ACL/SID buffers remain live through CreateFileW,
    // which copies the descriptor. The returned owning handle is transferred once.
    unsafe {
        let sid = (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let size = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>()
            + GetLengthSid(sid) as usize;
        let mut bytes = vec![0usize; size.div_ceil(size_of::<usize>())];
        let acl = bytes.as_mut_ptr().cast::<ACL>();
        if InitializeAcl(acl, size as u32, ACL_REVISION) == 0
            || AddAccessAllowedAce(acl, ACL_REVISION, 0x001f01ff, sid) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut descriptor: SECURITY_DESCRIPTOR = std::mem::zeroed();
        let sd = ptr::addr_of_mut!(descriptor).cast();
        if InitializeSecurityDescriptor(sd, 1) == 0
            || SetSecurityDescriptorOwner(sd, sid, 0) == 0
            || SetSecurityDescriptorDacl(sd, 1, acl, 0) == 0
            || SetSecurityDescriptorControl(sd, SE_DACL_PROTECTED, SE_DACL_PROTECTED) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attrs = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = CreateFileW(
            path.as_ptr(),
            0x001f01ff,
            7,
            &attrs,
            1,
            128,
            ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let file = File::from_raw_handle(handle);
        validate(&file)?;
        Ok(file)
    }
}
pub fn validate(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let user = user()?;
    // SAFETY: the OS descriptor owns returned pointers and is freed after use.
    unsafe {
        let sid = (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let mut owner = ptr::null_mut();
        let mut acl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        let result = GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut descriptor,
        );
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result as i32));
        }
        let _descriptor = Descriptor(descriptor);
        let mut control = 0;
        let mut revision = 0;
        if owner.is_null()
            || EqualSid(owner, sid) == 0
            || acl.is_null()
            || GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) == 0
            || control & SE_DACL_PROTECTED == 0
            || (*acl).AceCount != 1
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let mut ace = ptr::null_mut();
        if GetAce(acl, 0, &mut ace) == 0 {
            return Err(io::Error::last_os_error());
        }
        let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
        if ace.Header.AceType != 0
            || ace.Header.AceFlags != 0
            || EqualSid(ptr::addr_of!(ace.SidStart).cast_mut().cast(), sid) == 0
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_acl_is_rejected_and_sealed_acl_is_accepted() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(validate(file.as_file()).is_err());
        let private = tempfile::Builder::new().make(create).unwrap();
        validate(private.as_file()).unwrap();
    }
    #[test]
    fn os_denies_read_to_restricted_token_but_owner_can_read() {
        let private = tempfile::Builder::new().make(create).unwrap();
        assert!(File::open(private.path()).is_ok());
        // This is an actual kernel access check using an impersonated restricted
        // token, not a claim that a second logged-in account was exercised.
        unsafe {
            let mut raw = ptr::null_mut();
            assert_ne!(
                OpenProcessToken(
                    GetCurrentProcess(),
                    TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY,
                    &mut raw
                ),
                0
            );
            let token = Token(raw);
            let mut world = [0usize; 16];
            let mut size = size_of_val(&world) as u32;
            assert_ne!(
                CreateWellKnownSid(
                    WinWorldSid,
                    ptr::null_mut(),
                    world.as_mut_ptr().cast(),
                    &mut size
                ),
                0
            );
            let restrict = SID_AND_ATTRIBUTES {
                Sid: world.as_mut_ptr().cast(),
                Attributes: 0,
            };
            let mut restricted = ptr::null_mut();
            assert_ne!(
                CreateRestrictedToken(
                    token.0,
                    DISABLE_MAX_PRIVILEGE,
                    0,
                    ptr::null(),
                    0,
                    ptr::null(),
                    1,
                    &restrict,
                    &mut restricted
                ),
                0
            );
            let restricted = Token(restricted);
            assert_ne!(ImpersonateLoggedOnUser(restricted.0), 0);
            let opened = File::open(private.path());
            let reverted = RevertToSelf();
            assert_ne!(reverted, 0);
            assert_eq!(opened.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        }
        assert!(File::open(private.path()).is_ok());
    }
}
