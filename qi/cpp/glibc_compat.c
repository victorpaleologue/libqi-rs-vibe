/* Compatibility of modules with the glibc of old robots (NAOqi 2.1).
 *
 * The standard library of Rust calls `getauxval`, which appeared in glibc 2.16: a library
 * referencing it does not load with an older glibc. This definition, hidden (it is not exported,
 * and only resolves the references of the library itself), calls the one of the C library when
 * it has one, and otherwise answers 0, the value of an absent entry, which the standard library
 * handles. */
#define _GNU_SOURCE
#include <dlfcn.h>

typedef unsigned long (*getauxval_function)(unsigned long);

__attribute__((visibility("hidden"))) unsigned long getauxval(unsigned long type) {
  static getauxval_function real = 0;
  static int resolved = 0;
  if (!resolved) {
    real = (getauxval_function)dlsym(RTLD_DEFAULT, "getauxval");
    resolved = 1;
  }
  return real ? real(type) : 0;
}
