// We need to forward routine registration from C to Rust
// to avoid the linker removing the static library.

void R_init_tabellion_extendr(void *dll);

void R_init_tabellion(void *dll) {
    R_init_tabellion_extendr(dll);
}
