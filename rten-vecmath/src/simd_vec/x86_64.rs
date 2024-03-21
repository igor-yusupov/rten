use std::arch::x86_64::{
    __m256, __m256i, _mm256_add_epi32, _mm256_add_ps, _mm256_andnot_ps, _mm256_blendv_epi8,
    _mm256_blendv_ps, _mm256_castsi256_ps, _mm256_cmp_ps, _mm256_cmpgt_epi32, _mm256_cvttps_epi32,
    _mm256_div_ps, _mm256_fmadd_ps, _mm256_loadu_ps, _mm256_loadu_si256, _mm256_max_ps,
    _mm256_mul_ps, _mm256_set1_epi32, _mm256_set1_ps, _mm256_setzero_si256, _mm256_slli_epi32,
    _mm256_storeu_ps, _mm256_storeu_si256, _mm256_sub_epi32, _mm256_sub_ps, _mm_prefetch,
    _CMP_GE_OQ, _CMP_LE_OQ, _CMP_LT_OQ, _MM_HINT_ET0, _MM_HINT_T0,
};

use crate::simd_vec::{SimdFloat, SimdInt};

impl SimdInt for __m256i {
    type Float = __m256;
    type Mask = __m256i;

    const LEN: usize = 8;

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn zero() -> Self {
        _mm256_setzero_si256()
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn splat(val: i32) -> Self {
        _mm256_set1_epi32(val)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn gt(self, other: Self) -> Self::Mask {
        _mm256_cmpgt_epi32(self, other)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn blend(self, other: Self, mask: Self::Mask) -> Self {
        _mm256_blendv_epi8(self, other, mask)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn add(self, rhs: Self) -> Self {
        _mm256_add_epi32(self, rhs)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn sub(self, rhs: Self) -> Self {
        _mm256_sub_epi32(self, rhs)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn shl<const COUNT: i32>(self) -> Self {
        _mm256_slli_epi32(self, COUNT)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn reinterpret_as_float(self) -> Self::Float {
        _mm256_castsi256_ps(self)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn load(ptr: *const i32) -> Self {
        // Cast is OK because instruction does not require alignment.
        _mm256_loadu_si256(ptr as *const __m256i)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn store(self, ptr: *mut i32) {
        // Cast is OK because instruction does not require alignment.
        _mm256_storeu_si256(ptr as *mut __m256i, self)
    }
}

impl SimdFloat for __m256 {
    type Int = __m256i;
    type Mask = __m256;

    const LEN: usize = 8;

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn splat(val: f32) -> Self {
        _mm256_set1_ps(val)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn abs(self) -> Self {
        // https://stackoverflow.com/q/63599391/434243
        let sign_bit = _mm256_set1_ps(-0.0);
        _mm256_andnot_ps(sign_bit, self)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn mul_add(self, a: Self, b: Self) -> Self {
        _mm256_fmadd_ps(self, a, b)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn sub(self, rhs: Self) -> Self {
        _mm256_sub_ps(self, rhs)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn add(self, rhs: Self) -> Self {
        _mm256_add_ps(self, rhs)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn to_int_trunc(self) -> Self::Int {
        _mm256_cvttps_epi32(self)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn mul(self, rhs: Self) -> Self {
        _mm256_mul_ps(self, rhs)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn div(self, rhs: Self) -> Self {
        _mm256_div_ps(self, rhs)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn ge(self, rhs: Self::Mask) -> Self {
        _mm256_cmp_ps(self, rhs, _CMP_GE_OQ)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn le(self, rhs: Self::Mask) -> Self {
        _mm256_cmp_ps(self, rhs, _CMP_LE_OQ)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn lt(self, rhs: Self::Mask) -> Self {
        _mm256_cmp_ps(self, rhs, _CMP_LT_OQ)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn max(self, rhs: Self) -> Self {
        _mm256_max_ps(self, rhs)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn blend(self, rhs: Self, mask: Self::Mask) -> Self {
        _mm256_blendv_ps(self, rhs, mask)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn load(ptr: *const f32) -> Self {
        _mm256_loadu_ps(ptr)
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    #[target_feature(enable = "fma")]
    unsafe fn store(self, ptr: *mut f32) {
        _mm256_storeu_ps(ptr, self)
    }

    /// Prefetch the cache line containing `data`, for reading.
    #[inline]
    unsafe fn prefetch(data: *const f32) {
        _mm_prefetch(data as *const i8, _MM_HINT_T0);
    }

    /// Prefetch the cache line containing `data`, for writing.
    #[inline]
    unsafe fn prefetch_write(data: *mut f32) {
        _mm_prefetch(data as *const i8, _MM_HINT_ET0);
    }
}
