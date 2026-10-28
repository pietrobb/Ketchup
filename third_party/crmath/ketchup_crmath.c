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
float cr_sinf(float);
float cr_cosf(float);
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
float sinf(float x) { return cr_sinf(x); }
float cosf(float x) { return cr_cosf(x); }
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

#if defined(_WIN32)
/* Rust's f64::hypot calls the CRT's _hypot on Windows. */
double _hypot(double x, double y) { return cr_hypot(x, y); }

/* The UCRT headers declare acosh, cbrt, hypot, log2, powf, ... as dllimport,
 * so OCCT calls them through the import pointer __imp_<name>. Defining those
 * pointers here routes the calls to the functions above and keeps the linker
 * from pulling ucrt.lib's member, which would define <name> a second time. */
#define KETCHUP_IMPORT(ret, name, params) ret (*__imp_##name) params = name;
KETCHUP_IMPORT(double, sin, (double))
KETCHUP_IMPORT(double, cos, (double))
KETCHUP_IMPORT(double, tan, (double))
KETCHUP_IMPORT(double, asin, (double))
KETCHUP_IMPORT(double, acos, (double))
KETCHUP_IMPORT(double, atan, (double))
KETCHUP_IMPORT(double, atan2, (double, double))
KETCHUP_IMPORT(double, sinh, (double))
KETCHUP_IMPORT(double, cosh, (double))
KETCHUP_IMPORT(double, tanh, (double))
KETCHUP_IMPORT(double, asinh, (double))
KETCHUP_IMPORT(double, acosh, (double))
KETCHUP_IMPORT(double, atanh, (double))
KETCHUP_IMPORT(double, exp, (double))
KETCHUP_IMPORT(double, exp2, (double))
KETCHUP_IMPORT(double, log, (double))
KETCHUP_IMPORT(double, log2, (double))
KETCHUP_IMPORT(double, log10, (double))
KETCHUP_IMPORT(double, log1p, (double))
KETCHUP_IMPORT(double, pow, (double, double))
KETCHUP_IMPORT(double, cbrt, (double))
KETCHUP_IMPORT(double, hypot, (double, double))
KETCHUP_IMPORT(double, _hypot, (double, double))
KETCHUP_IMPORT(float, sinf, (float))
KETCHUP_IMPORT(float, cosf, (float))
KETCHUP_IMPORT(float, acosf, (float))
KETCHUP_IMPORT(float, powf, (float, float))
KETCHUP_IMPORT(float, tanf, (float))
#endif
