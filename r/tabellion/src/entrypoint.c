#include <R_ext/Rdynload.h>
#include <R_ext/Visibility.h>

extern void R_init_tabellion_extendr(DllInfo *dll);

void attribute_visible R_init_tabellion(DllInfo *dll) {
    R_init_tabellion_extendr(dll);
}
