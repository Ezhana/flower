use std::panic::{catch_unwind, AssertUnwindSafe};

use flower_core::Calculator;

/// C/C++ 看见的只是一个 opaque handle。
#[repr(C)]
pub struct DemoCalculator {
    _private: [u8; 0],
}

/// FFI 层统一错误码。
#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DemoStatus {
    Ok = 0,
    NullPointer = 1,
    InvalidArgument = 2,
    InternalPanic = 3,
}

#[inline]
unsafe fn as_calculator<'a>(
    handle: *mut DemoCalculator,
) -> Result<&'a mut Calculator, DemoStatus> {
    if handle.is_null() {
        return Err(DemoStatus::NullPointer);
    }

    // SAFETY:
    // handle 必须来自 demo_calculator_create，
    // 且在 demo_calculator_destroy 后不得再次使用。
    Ok(&mut *(handle.cast::<Calculator>()))
}

/// 创建对象。
///
/// Rust:
///     Box<Calculator>
///       ↓
///     *mut Calculator
///       ↓
///     *mut DemoCalculator
#[unsafe(no_mangle)]
pub extern "C" fn demo_calculator_create(
    initial: f64,
) -> *mut DemoCalculator {
    match catch_unwind(AssertUnwindSafe(|| {
        Box::into_raw(Box::new(
            Calculator::new(initial)
        ))
            .cast::<DemoCalculator>()
    })) {
        Ok(ptr) => ptr,
        Err(_) => std::ptr::null_mut(),
    }
}

/// 销毁对象。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_calculator_destroy(
    handle: *mut DemoCalculator,
) {
    if handle.is_null() {
        return;
    }

    // SAFETY:
    // handle 必须是 create 返回的有效指针，并且只能 destroy 一次。
    drop(Box::from_raw(
        handle.cast::<Calculator>()
    ));
}

/// add
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_calculator_add(
    handle: *mut DemoCalculator,
    value: f64,
) -> DemoStatus {
    if !value.is_finite() {
        return DemoStatus::InvalidArgument;
    }

    match catch_unwind(AssertUnwindSafe(|| {
        let calc = as_calculator(handle)?;

        calc.add(value);

        Ok::<(), DemoStatus>(())
    })) {
        Ok(Ok(())) => DemoStatus::Ok,
        Ok(Err(status)) => status,
        Err(_) => DemoStatus::InternalPanic,
    }
}

/// multiply
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_calculator_multiply(
    handle: *mut DemoCalculator,
    value: f64,
) -> DemoStatus {
    if !value.is_finite() {
        return DemoStatus::InvalidArgument;
    }

    match catch_unwind(AssertUnwindSafe(|| {
        let calc = as_calculator(handle)?;

        calc.multiply(value);

        Ok::<(), DemoStatus>(())
    })) {
        Ok(Ok(())) => DemoStatus::Ok,
        Ok(Err(status)) => status,
        Err(_) => DemoStatus::InternalPanic,
    }
}

/// 折扣
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_calculator_discount(
    handle: *mut DemoCalculator,
    percent: f64,
) -> DemoStatus {
    if !percent.is_finite() {
        return DemoStatus::InvalidArgument;
    }

    match catch_unwind(AssertUnwindSafe(|| {
        let calc = as_calculator(handle)?;

        calc.apply_discount(percent)
            .map_err(|_| DemoStatus::InvalidArgument)
    })) {
        Ok(Ok(())) => DemoStatus::Ok,
        Ok(Err(status)) => status,
        Err(_) => DemoStatus::InternalPanic,
    }
}

/// 获取当前值。
///
/// 采用 out parameter，而不是直接返回复杂 Rust 类型。
#[unsafe(no_mangle)]
pub unsafe extern "C" fn demo_calculator_value(
    handle: *const DemoCalculator,
    out_value: *mut f64,
) -> DemoStatus {
    if handle.is_null() || out_value.is_null() {
        return DemoStatus::NullPointer;
    }

    match catch_unwind(AssertUnwindSafe(|| {
        // SAFETY:
        // handle 和 out_value 已经检查过非空。
        let calc = &*handle.cast::<Calculator>();

        *out_value = calc.value();

        DemoStatus::Ok
    })) {
        Ok(status) => status,
        Err(_) => DemoStatus::InternalPanic,
    }
}

/// 将错误码转换成静态 C 字符串。
#[unsafe(no_mangle)]
pub extern "C" fn demo_status_string(
    status: DemoStatus,
) -> *const std::ffi::c_char {
    match status {
        DemoStatus::Ok =>
            c"ok".as_ptr(),

        DemoStatus::NullPointer =>
            c"null pointer".as_ptr(),

        DemoStatus::InvalidArgument =>
            c"invalid argument".as_ptr(),

        DemoStatus::InternalPanic =>
            c"internal panic".as_ptr(),
    }
}