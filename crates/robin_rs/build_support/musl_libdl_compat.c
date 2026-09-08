/*
 * MUSL implements the dlopen family in libc and does not need a separate
 * libdl. This one unused symbol makes a conventional compatibility archive
 * available to dependencies which still emit `-ldl` for every Linux target.
 */
void robin_projection_musl_libdl_compatibility_archive(void) {}
