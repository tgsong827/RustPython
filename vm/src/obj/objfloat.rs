use num_bigint::{BigInt, ToBigInt};
use num_complex::Complex64;
use num_rational::Ratio;
use num_traits::{pow, ToPrimitive, Zero};

use super::objbytes::PyBytes;
use super::objint::{self, PyInt, PyIntRef};
use super::objstr::{PyString, PyStringRef};
use super::objtype::PyClassRef;
use crate::format::FormatSpec;
use crate::function::{OptionalArg, OptionalOption};
use crate::pyobject::{
    BorrowValue, IntoPyObject,
    PyArithmaticValue::{self, *},
    PyClassImpl, PyComparisonValue, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;
use rustpython_common::{float_ops, hash};

/// Convert a string or number to a floating point number, if possible.
#[pyclass(module = false, name = "float")]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PyFloat {
    value: f64,
}

impl PyFloat {
    pub fn to_f64(self) -> f64 {
        self.value
    }
}

impl PyValue for PyFloat {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.float_type()
    }
}

impl IntoPyObject for f64 {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_float(self)
    }
}
impl IntoPyObject for f32 {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_float(f64::from(self))
    }
}

impl From<f64> for PyFloat {
    fn from(value: f64) -> Self {
        PyFloat { value }
    }
}

pub fn try_float(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<f64>> {
    let v = if let Some(float) = obj.payload_if_subclass::<PyFloat>(vm) {
        Some(float.value)
    } else if let Some(int) = obj.payload_if_subclass::<PyInt>(vm) {
        Some(objint::try_float(int.borrow_value(), vm)?)
    } else {
        None
    };
    Ok(v)
}

macro_rules! impl_try_from_object_float {
    ($($t:ty),*) => {
        $(impl TryFromObject for $t {
            fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
                PyFloatRef::try_from_object(vm, obj).map(|f| f.to_f64() as $t)
            }
        })*
    };
}

impl_try_from_object_float!(f32, f64);

fn inner_div(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    float_ops::div(v1, v2)
        .ok_or_else(|| vm.new_zero_division_error("float division by zero".to_owned()))
}

fn inner_mod(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    float_ops::mod_(v1, v2)
        .ok_or_else(|| vm.new_zero_division_error("float mod by zero".to_owned()))
}

pub fn try_bigint(value: f64, vm: &VirtualMachine) -> PyResult<BigInt> {
    match value.to_bigint() {
        Some(int) => Ok(int),
        None => {
            if value.is_infinite() {
                Err(vm.new_overflow_error(
                    "OverflowError: cannot convert float infinity to integer".to_owned(),
                ))
            } else if value.is_nan() {
                Err(vm
                    .new_value_error("ValueError: cannot convert float NaN to integer".to_owned()))
            } else {
                // unreachable unless BigInt has a bug
                unreachable!(
                    "A finite float value failed to be converted to bigint: {}",
                    value
                )
            }
        }
    }
}

fn inner_floordiv(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    float_ops::floordiv(v1, v2)
        .ok_or_else(|| vm.new_zero_division_error("float floordiv by zero".to_owned()))
}

fn inner_divmod(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<(f64, f64)> {
    float_ops::divmod(v1, v2).ok_or_else(|| vm.new_zero_division_error("float divmod()".to_owned()))
}

pub fn float_pow(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult {
    if v1.is_zero() {
        let msg = format!("{} cannot be raised to a negative power", v1);
        Err(vm.new_zero_division_error(msg))
    } else if v1.is_sign_negative() && (v2.floor() - v2).abs() > f64::EPSILON {
        let v1 = Complex64::new(v1, 0.);
        let v2 = Complex64::new(v2, 0.);
        Ok(v1.powc(v2).into_pyobject(vm))
    } else {
        Ok(v1.powf(v2).into_pyobject(vm))
    }
}

#[pyimpl(flags(BASETYPE))]
#[allow(clippy::trivially_copy_pass_by_ref)]
impl PyFloat {
    #[pyslot]
    fn tp_new(
        cls: PyClassRef,
        arg: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyFloatRef> {
        let float_val = match arg {
            OptionalArg::Present(val) => to_float(vm, &val),
            OptionalArg::Missing => Ok(0f64),
        };
        PyFloat::from(float_val?).into_ref_with_type(vm, cls)
    }

    #[inline]
    fn cmp<F, G>(
        &self,
        other: PyObjectRef,
        float_op: F,
        int_op: G,
        vm: &VirtualMachine,
    ) -> PyComparisonValue
    where
        F: Fn(f64, f64) -> bool,
        G: Fn(f64, &BigInt) -> bool,
    {
        if let Some(other) = other.payload_if_subclass::<PyFloat>(vm) {
            Implemented(float_op(self.value, other.value))
        } else if let Some(other) = other.payload_if_subclass::<PyInt>(vm) {
            Implemented(int_op(self.value, other.borrow_value()))
        } else {
            NotImplemented
        }
    }

    #[pymethod(name = "__format__")]
    fn format(&self, spec: PyStringRef, vm: &VirtualMachine) -> PyResult<String> {
        match FormatSpec::parse(spec.borrow_value())
            .and_then(|format_spec| format_spec.format_float(self.value))
        {
            Ok(string) => Ok(string),
            Err(err) => Err(vm.new_value_error(err.to_string())),
        }
    }

    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a == b, float_ops::eq_int, vm)
    }

    #[pymethod(name = "__ne__")]
    fn ne(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.eq(other, vm).map(|v| !v)
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a < b, float_ops::lt_int, vm)
    }

    #[pymethod(name = "__le__")]
    fn le(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(
            other,
            |a, b| a <= b,
            |a, b| {
                if let (Some(a_int), Some(b_float)) = (a.to_bigint(), b.to_f64()) {
                    a <= b_float && a_int <= *b
                } else {
                    float_ops::lt_int(a, b)
                }
            },
            vm,
        )
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(other, |a, b| a > b, float_ops::gt_int, vm)
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyComparisonValue {
        self.cmp(
            other,
            |a, b| a >= b,
            |a, b| {
                if let (Some(a_int), Some(b_float)) = (a.to_bigint(), b.to_f64()) {
                    a >= b_float && a_int >= *b
                } else {
                    float_ops::gt_int(a, b)
                }
            },
            vm,
        )
    }

    #[pymethod(name = "__abs__")]
    fn abs(&self) -> f64 {
        self.value.abs()
    }

    #[inline]
    fn simple_op<F>(
        &self,
        other: PyObjectRef,
        op: F,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<f64>>
    where
        F: Fn(f64, f64) -> PyResult<f64>,
    {
        try_float(&other, vm)?.map_or_else(
            || Ok(NotImplemented),
            |other| Ok(Implemented(op(self.value, other)?)),
        )
    }

    #[inline]
    fn complex_op<F>(&self, other: PyObjectRef, op: F, vm: &VirtualMachine) -> PyResult
    where
        F: Fn(f64, f64) -> PyResult,
    {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| op(self.value, other),
        )
    }

    #[inline]
    fn tuple_op<F>(
        &self,
        other: PyObjectRef,
        op: F,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<(f64, f64)>>
    where
        F: Fn(f64, f64) -> PyResult<(f64, f64)>,
    {
        try_float(&other, vm)?.map_or_else(
            || Ok(NotImplemented),
            |other| Ok(Implemented(op(self.value, other)?)),
        )
    }

    #[pymethod(name = "__add__")]
    #[pymethod(name = "__radd__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| Ok(a + b), vm)
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self) -> bool {
        self.value != 0.0
    }

    #[pymethod(name = "__divmod__")]
    fn divmod(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<(f64, f64)>> {
        self.tuple_op(other, |a, b| inner_divmod(a, b, vm), vm)
    }

    #[pymethod(name = "__rdivmod__")]
    fn rdivmod(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<(f64, f64)>> {
        self.tuple_op(other, |a, b| inner_divmod(b, a, vm), vm)
    }

    #[pymethod(name = "__floordiv__")]
    fn floordiv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| inner_floordiv(a, b, vm), vm)
    }

    #[pymethod(name = "__rfloordiv__")]
    fn rfloordiv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| inner_floordiv(b, a, vm), vm)
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| inner_mod(a, b, vm), vm)
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| inner_mod(b, a, vm), vm)
    }

    #[pymethod(name = "__pos__")]
    fn pos(&self) -> f64 {
        self.value
    }

    #[pymethod(name = "__neg__")]
    fn neg(&self) -> f64 {
        -self.value
    }

    #[pymethod(name = "__pow__")]
    fn pow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.complex_op(other, |a, b| float_pow(a, b, vm), vm)
    }

    #[pymethod(name = "__rpow__")]
    fn rpow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.complex_op(other, |a, b| float_pow(b, a, vm), vm)
    }

    #[pymethod(name = "__sub__")]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| Ok(a - b), vm)
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| Ok(b - a), vm)
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self) -> String {
        float_ops::to_string(self.value)
    }

    #[pymethod(name = "__truediv__")]
    fn truediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| inner_div(a, b, vm), vm)
    }

    #[pymethod(name = "__rtruediv__")]
    fn rtruediv(
        &self,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| inner_div(b, a, vm), vm)
    }

    #[pymethod(name = "__mul__")]
    #[pymethod(name = "__rmul__")]
    fn mul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyArithmaticValue<f64>> {
        self.simple_op(other, |a, b| Ok(a * b), vm)
    }

    #[pymethod(name = "__trunc__")]
    fn trunc(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        try_bigint(self.value, vm)
    }

    #[pymethod(name = "__round__")]
    fn round(&self, ndigits: OptionalOption<PyIntRef>, vm: &VirtualMachine) -> PyResult {
        let ndigits = ndigits.flatten();
        let value = if let Some(ndigits) = ndigits {
            let ndigits = ndigits.borrow_value();
            if ndigits.is_zero() {
                let fract = self.value.fract();
                let value = if (fract.abs() - 0.5).abs() < std::f64::EPSILON {
                    if self.value.trunc() % 2.0 == 0.0 {
                        self.value - fract
                    } else {
                        self.value + fract
                    }
                } else {
                    self.value.round()
                };
                vm.ctx.new_float(value)
            } else {
                let ndigits = match ndigits {
                    ndigits if *ndigits > i32::max_value().to_bigint().unwrap() => i32::max_value(),
                    ndigits if *ndigits < i32::min_value().to_bigint().unwrap() => i32::min_value(),
                    _ => ndigits.to_i32().unwrap(),
                };
                if (self.value > 1e+16_f64 && ndigits >= 0i32)
                    || (ndigits + self.value.log10().floor() as i32 > 16i32)
                {
                    return Ok(vm.ctx.new_float(self.value));
                }
                if ndigits >= 0i32 {
                    vm.ctx.new_float(
                        (self.value * pow(10.0, ndigits as usize)).round()
                            / pow(10.0, ndigits as usize),
                    )
                } else {
                    let result = (self.value / pow(10.0, (-ndigits) as usize)).round()
                        * pow(10.0, (-ndigits) as usize);
                    if result.is_nan() {
                        return Ok(vm.ctx.new_float(0.0));
                    }
                    vm.ctx.new_float(result)
                }
            }
        } else {
            let fract = self.value.fract();
            let value = if (fract.abs() - 0.5).abs() < std::f64::EPSILON {
                if self.value.trunc() % 2.0 == 0.0 {
                    self.value - fract
                } else {
                    self.value + fract
                }
            } else {
                self.value.round()
            };
            let int = try_bigint(value, vm)?;
            vm.ctx.new_int(int)
        };
        Ok(value)
    }

    #[pymethod(name = "__int__")]
    fn int(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        self.trunc(vm)
    }

    #[pymethod(name = "__float__")]
    fn float(zelf: PyRef<Self>) -> PyFloatRef {
        zelf
    }

    #[pymethod(name = "__hash__")]
    fn hash(&self) -> hash::PyHash {
        hash::hash_float(self.value)
    }

    #[pyproperty]
    fn real(zelf: PyRef<Self>) -> PyFloatRef {
        zelf
    }

    #[pyproperty]
    fn imag(&self) -> f64 {
        0.0f64
    }

    #[pymethod(name = "conjugate")]
    fn conjugate(zelf: PyRef<Self>) -> PyFloatRef {
        zelf
    }

    #[pymethod(name = "is_integer")]
    fn is_integer(&self) -> bool {
        float_ops::is_integer(self.value)
    }

    #[pymethod(name = "as_integer_ratio")]
    fn as_integer_ratio(&self, vm: &VirtualMachine) -> PyResult {
        let value = self.value;
        if !value.is_finite() {
            return Err(if value.is_infinite() {
                vm.new_overflow_error("cannot convert Infinity to integer ratio".to_owned())
            } else if value.is_nan() {
                vm.new_value_error("cannot convert NaN to integer ratio".to_owned())
            } else {
                unreachable!("it must be finite")
            });
        }

        let ratio = Ratio::from_float(value).unwrap();
        let numer = vm.ctx.new_bigint(ratio.numer());
        let denom = vm.ctx.new_bigint(ratio.denom());
        Ok(vm.ctx.new_tuple(vec![numer, denom]))
    }

    #[pymethod]
    fn fromhex(repr: PyStringRef, vm: &VirtualMachine) -> PyResult<f64> {
        float_ops::from_hex(repr.borrow_value().trim()).ok_or_else(|| {
            vm.new_value_error("invalid hexadecimal floating-point string".to_owned())
        })
    }

    #[pymethod]
    fn hex(&self) -> String {
        float_ops::to_hex(self.value)
    }
}

fn to_float(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<f64> {
    let value = if let Some(float) = obj.payload_if_subclass::<PyFloat>(vm) {
        float.value
    } else if let Some(int) = obj.payload_if_subclass::<PyInt>(vm) {
        objint::try_float(int.borrow_value(), vm)?
    } else if let Some(s) = obj.payload_if_subclass::<PyString>(vm) {
        float_ops::parse_str(s.borrow_value().trim()).ok_or_else(|| {
            vm.new_value_error(format!("could not convert string to float: '{}'", s))
        })?
    } else if let Some(bytes) = obj.payload_if_subclass::<PyBytes>(vm) {
        lexical_core::parse(bytes.borrow_value()).map_err(|_| {
            vm.new_value_error(format!(
                "could not convert string to float: '{}'",
                bytes.repr()
            ))
        })?
    } else {
        let method = vm.get_method_or_type_error(obj.clone(), "__float__", || {
            format!(
                "float() argument must be a string or a number, not '{}'",
                obj.lease_class().name
            )
        })?;
        let result = vm.invoke(&method, vec![])?;
        PyFloatRef::try_from_object(vm, result)?.to_f64()
    };
    Ok(value)
}

pub type PyFloatRef = PyRef<PyFloat>;

// Retrieve inner float value:
pub fn get_value(obj: &PyObjectRef) -> f64 {
    obj.payload::<PyFloat>().unwrap().value
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct IntoPyFloat {
    value: f64,
}

impl IntoPyFloat {
    pub fn to_f64(self) -> f64 {
        self.value
    }
}

impl TryFromObject for IntoPyFloat {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(IntoPyFloat {
            value: to_float(vm, &obj)?,
        })
    }
}

#[rustfmt::skip] // to avoid line splitting
pub fn init(context: &PyContext) {
    PyFloat::extend_class(context, &context.types.float_type);
}
