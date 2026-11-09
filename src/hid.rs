//! Separate overlapped reader/writer. Pending buffers live until completed cancellation.
use crate::win::{Handle, error, wide};
use std::{
    io,
    mem::size_of,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::{
        Devices::{DeviceAndDriverInstallation::*, HumanInterfaceDevice::*},
        Foundation::*,
        Storage::FileSystem::*,
        System::{IO::*, Threading::*},
    },
    core::GUID,
};
#[derive(Debug, Clone)]
pub struct Interface {
    pub path: String,
    pub vid: u16,
    pub pid: u16,
    pub page: u16,
    pub usage: u16,
    pub input_len: u16,
    pub output_len: u16,
    pub feature_len: u16,
    pub product: String,
}
struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {
    fn drop(&mut self) {
        // SAFETY: uniquely owns this SetupAPI device information set.
        unsafe { SetupDiDestroyDeviceInfoList(self.0) };
    }
}
fn open(path: &str, access: u32, flags: u32) -> io::Result<Handle> {
    // SAFETY: terminated UTF-16 path lives through the call.
    unsafe {
        Handle::new(CreateFileW(
            wide(path).as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            flags,
            ptr::null_mut(),
        ))
    }
}
pub fn enumerate() -> io::Result<Vec<Interface>> {
    // SAFETY: sized output structures and aligned buffers with explicit byte capacities.
    unsafe {
        let mut guid = GUID::from_u128(0);
        HidD_GetHidGuid(&mut guid);
        let set = SetupDiGetClassDevsW(
            &guid,
            ptr::null(),
            ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        );
        if set == -1 {
            return Err(error("Enumerate HID devices"));
        }
        let set = DeviceSet(set);
        let mut out = Vec::new();
        for index in 0.. {
            let mut data = SP_DEVICE_INTERFACE_DATA {
                cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInterfaces(set.0, ptr::null(), &guid, index, &mut data) == 0 {
                if GetLastError() == ERROR_NO_MORE_ITEMS {
                    break;
                }
                return Err(error("Enumerate HID interface"));
            }
            let mut needed = 0;
            SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &data,
                ptr::null_mut(),
                0,
                &mut needed,
                ptr::null_mut(),
            );
            if needed < 6 {
                continue;
            }
            let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
            let detail = storage
                .as_mut_ptr()
                .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &data,
                detail,
                needed,
                ptr::null_mut(),
                ptr::null_mut(),
            ) == 0
            {
                continue;
            }
            let path_ptr = ptr::addr_of!((*detail).DevicePath).cast::<u16>();
            let capacity = (needed as usize - 4) / 2;
            let chars = std::slice::from_raw_parts(path_ptr, capacity);
            let len = chars.iter().position(|c| *c == 0).unwrap_or(capacity);
            let path = String::from_utf16_lossy(&chars[..len]);
            if let Ok(iface) = inspect(&path) {
                out.push(iface);
            }
        }
        Ok(out)
    }
}
fn inspect(path: &str) -> io::Result<Interface> {
    let h = open(path, 0, 0)?;
    // SAFETY: valid handle, initialized output structs; preparsed allocation is freed.
    unsafe {
        let mut a = HIDD_ATTRIBUTES {
            Size: size_of::<HIDD_ATTRIBUTES>() as u32,
            ..Default::default()
        };
        if !HidD_GetAttributes(h.raw(), &mut a) {
            return Err(error("HID attributes"));
        }
        let mut prep = 0;
        if !HidD_GetPreparsedData(h.raw(), &mut prep) {
            return Err(error("HID descriptor"));
        }
        let mut caps = HIDP_CAPS::default();
        let status = HidP_GetCaps(prep, &mut caps);
        HidD_FreePreparsedData(prep);
        if status != HIDP_STATUS_SUCCESS {
            return Err(io::Error::other("Invalid HID capabilities"));
        }
        let mut product = [0u16; 256];
        HidD_GetProductString(
            h.raw(),
            product.as_mut_ptr().cast(),
            size_of_val(&product) as u32,
        );
        let len = product
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(product.len());
        Ok(Interface {
            path: path.into(),
            vid: a.VendorID,
            pid: a.ProductID,
            page: caps.UsagePage,
            usage: caps.Usage,
            input_len: caps.InputReportByteLength,
            output_len: caps.OutputReportByteLength,
            feature_len: caps.FeatureReportByteLength,
            product: String::from_utf16_lossy(&product[..len]),
        })
    }
}
pub struct Transport {
    reader: Handle,
    writer: Handle,
    event: Handle,
    input_len: usize,
}
impl Transport {
    pub fn open(i: &Interface) -> io::Result<Self> {
        let reader = open(&i.path, GENERIC_READ, FILE_FLAG_OVERLAPPED)?;
        let writer = open(&i.path, GENERIC_READ | GENERIC_WRITE, FILE_FLAG_OVERLAPPED)?;
        // SAFETY: creates an unnamed manual-reset event without a security descriptor.
        let event = unsafe { Handle::new(CreateEventW(ptr::null(), 1, 0, ptr::null()))? };
        Ok(Self {
            reader,
            writer,
            event,
            input_len: usize::from(i.input_len),
        })
    }
    fn finish(&self, h: HANDLE, ov: &mut OVERLAPPED, timeout: Duration) -> io::Result<Option<u32>> {
        // SAFETY: caller keeps buffer and OVERLAPPED alive; cancellation is followed
        // by a completion wait so no kernel operation retains a freed address.
        unsafe {
            let wait = WaitForSingleObject(
                self.event.raw(),
                timeout.as_millis().max(1).min(u128::from(u32::MAX - 1)) as u32,
            );
            let mut count = 0;
            if wait == WAIT_OBJECT_0 {
                if GetOverlappedResult(h, ov, &mut count, 0) != 0 {
                    return Ok(Some(count));
                }
                return Err(error("Complete HID I/O"));
            }
            CancelIoEx(h, ov);
            GetOverlappedResult(h, ov, &mut count, 1);
            if wait == WAIT_TIMEOUT {
                Ok(None)
            } else {
                Err(error("Wait for HID I/O"))
            }
        }
    }
    pub fn read(&self, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        if self.input_len == 0 {
            return Ok(None);
        }
        let mut buffer = vec![0; self.input_len];
        let mut ov = OVERLAPPED {
            hEvent: self.event.raw(),
            ..Default::default()
        };
        let mut count = 0;
        // SAFETY: buffer and OVERLAPPED live until completion or completed cancellation.
        unsafe {
            ResetEvent(self.event.raw());
            if ReadFile(
                self.reader.raw(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                &mut count,
                &mut ov,
            ) == 0
            {
                if GetLastError() != ERROR_IO_PENDING {
                    return Err(error("Read HID report"));
                }
                match self.finish(self.reader.raw(), &mut ov, timeout)? {
                    Some(n) => count = n,
                    None => return Ok(None),
                }
            }
        }
        buffer.truncate(count as usize);
        Ok(Some(buffer))
    }
    pub fn write(&self, buffer: &[u8]) -> io::Result<()> {
        let mut ov = OVERLAPPED {
            hEvent: self.event.raw(),
            ..Default::default()
        };
        let mut count = 0;
        // SAFETY: separate write handle, immutable buffer held until I/O completes.
        unsafe {
            ResetEvent(self.event.raw());
            // hidclass.h: HID_IN_CTL_CODE(101), equivalent to HidD_SetOutputReport.
            // These two allow-listed receivers use the control pipe. Unlike the
            // synchronous HidD helper, DeviceIoControl gives us cancellable I/O.
            const IOCTL_HID_SET_OUTPUT_REPORT: u32 = (0x0b << 16) | (101 << 2) | 1;
            if DeviceIoControl(
                self.writer.raw(),
                IOCTL_HID_SET_OUTPUT_REPORT,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                ptr::null_mut(),
                0,
                &mut count,
                &mut ov,
            ) != 0
            {
                return Ok(());
            }
            let code = GetLastError();
            if code == ERROR_IO_PENDING {
                return self
                    .finish(self.writer.raw(), &mut ov, Duration::from_millis(500))?
                    .map(|_| ())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "HID control query timed out")
                    });
            }
            if code != ERROR_INVALID_FUNCTION && code != ERROR_NOT_SUPPORTED {
                return Err(io::Error::from_raw_os_error(code as i32));
            }
            ResetEvent(self.event.raw());
            if WriteFile(
                self.writer.raw(),
                buffer.as_ptr(),
                buffer.len() as u32,
                &mut count,
                &mut ov,
            ) == 0
            {
                if GetLastError() != ERROR_IO_PENDING {
                    return Err(error("Write HID query"));
                }
                count = self
                    .finish(self.writer.raw(), &mut ov, Duration::from_millis(500))?
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "HID write timed out")
                    })?;
            }
        }
        if count as usize != buffer.len() {
            return Err(io::Error::other("Incomplete HID write"));
        }
        Ok(())
    }
    pub fn exchange<T>(
        &self,
        request: &[u8],
        decode: impl Fn(&[u8]) -> Option<T>,
        timeout: Duration,
    ) -> io::Result<Option<T>> {
        for _ in 0..32 {
            if self.read(Duration::from_millis(2))?.is_none() {
                break;
            }
        }
        self.write(request)?;
        let deadline = Instant::now() + timeout;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            if let Some(r) = self.read(remaining)? {
                if let Some(value) = decode(&r) {
                    return Ok(Some(value));
                }
            } else {
                break;
            }
        }
        Ok(None)
    }
}
