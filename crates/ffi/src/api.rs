use flower_core::Calculator;
use std::ffi::c_int;
use std::ptr;

#[repr(C)]
pub struct MyLibCalculator {
    inner: Calculator,
}

#[unsafe(no_mangle)]
pub extern "C" fn my_lib_calculator_new() -> *mut MyLibCalculator {
    let calculator = Box::new(MyLibCalculator {
        inner: Calculator::new(),
    });

    Box::into_raw(calculator)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn my_lib_calculator_free(ptr: *mut MyLibCalculator) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        drop(Box::from_raw(ptr));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn my_lib_calculator_add(
    ptr: *const MyLibCalculator,
    a: c_int,
    b: c_int,
    out: *mut c_int,
) -> c_int {
    if ptr.is_null() || out.is_null() {
        return -1;
    }

    let calculator = unsafe { &*ptr };

    let result = match calculator.inner.add(a, b) {
        Ok(value) => value,
        Err(_) => return -2,
    };

    unsafe {
        ptr::write(out, result);
    }

    0
}
