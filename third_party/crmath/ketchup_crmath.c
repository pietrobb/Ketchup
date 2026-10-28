/* Replaces the platform libm functions that OCCT and the exact path call with
 * CORE-MATH's correctly rounded ones. Correct rounding makes every result the
 * double nearest to the true value, so macOS and Windows compute the same bits
 * even though Apple's and Microsoft's libm differ in the last bit. Documents
 * pin exact results bit for bit (result_fingerprint), so this is what lets a
 * document evaluated on one platform evaluate on the other.
 *
 * The library is linked whole-archive into every OCCT library and every Rust
 * binary that uses ketchup-exact; a definition inside the link unit always
 * wins over the system library. Built by third_party/crmath/CMakeLists.txt.
 * Functions that IEEE 754 already defines exactly (sqrt, fmod, ldexp, modf,
 * nextafter, remainder, fma) need no replacement. */

double cr_sin(double);
double cr_cos(double);
double cr_tan(double);
double cr_asin(double);
double cr_acos(double);
double cr_atan(double);
double cr_atan2(double, double);
double cr_sinh(double);
double cr_cosh(double);
double cr_tanh(double);
double cr_asinh(double);
double cr_acosh(double);
double cr_atanh(double);
double cr_exp(double);
double cr_exp2(double);
double cr_exp10(double);
double cr_log(double);
double cr_log2(double);
double cr_log10(double);
double cr_log1p(double);
double cr_pow(double, double);
double cr_cbrt(double);
double cr_hypot(double, double);
void cr_sincos(double, double *, double *);
float cr_acosf(float);
float cr_powf(float, float);
float cr_tanf(float);

double sin(double x) { return cr_sin(x); }
double cos(double x) { return cr_cos(x); }
double tan(double x) { return cr_tan(x); }
double asin(double x) { return cr_asin(x); }
double acos(double x) { return cr_acos(x); }
double atan(double x) { return cr_atan(x); }
double atan2(double y, double x) { return cr_atan2(y, x); }
double sinh(double x) { return cr_sinh(x); }
double cosh(double x) { return cr_cosh(x); }
double tanh(double x) { return cr_tanh(x); }
double asinh(double x) { return cr_asinh(x); }
double acosh(double x) { return cr_acosh(x); }
double atanh(double x) { return cr_atanh(x); }
double exp(double x) { return cr_exp(x); }
double exp2(double x) { return cr_exp2(x); }
double log(double x) { return cr_log(x); }
double log2(double x) { return cr_log2(x); }
double log10(double x) { return cr_log10(x); }
double log1p(double x) { return cr_log1p(x); }
double pow(double x, double y) { return cr_pow(x, y); }
double cbrt(double x) { return cr_cbrt(x); }
double hypot(double x, double y) { return cr_hypot(x, y); }
float acosf(float x) { return cr_acosf(x); }
float powf(float x, float y) { return cr_powf(x, y); }
float tanf(float x) { return cr_tanf(x); }

#if defined(__APPLE__)
/* clang on Apple fuses sin(x) and cos(x) of the same argument into
 * __sincos_stret and spells exp10 as __exp10. */
struct ketchup_sincos {
    double sin;
    double cos;
};

struct ketchup_sincos __sincos_stret(double x) {
    struct ketchup_sincos result;
    cr_sincos(x, &result.sin, &result.cos);
    return result;
}

double __exp10(double x) { return cr_exp10(x); }
#endif
