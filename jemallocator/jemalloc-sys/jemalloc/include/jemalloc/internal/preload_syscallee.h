#include <stdarg.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>


extern void *memory_profiler_raw_mmap(void *addr,
                               size_t length,
                               int prot,
                               int flags,
                               int fildes,
                               off_t off);

extern int memory_profiler_raw_munmap(void *addr, size_t length);