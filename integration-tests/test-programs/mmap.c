#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>

void __attribute__ ((noinline)) foobar() {
    // Leaked, never touched.
    mmap( NULL, 123 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0 );
    usleep( 1000 );

    // Leaked, touched.
    char * a1 = mmap( NULL, 5 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0 );
    memset( a1, 0,  4096 );
    memset( a1 + 4 * 4096, 0,  4096 );
    usleep( 1000 );

    // Fully deallocated.
    char * a2 = mmap( NULL, 6 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0 );
    usleep( 1000 );

    // Partially deallocated (at the start).
    char * a3 = mmap( NULL, 7 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0 );
    usleep( 1000 );

    // Partially deallocated (at the end).
    char * a4 = mmap( NULL, 7 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0 );
    usleep( 1000 );

    // Partially deallocated (in the middle).
    char * a5 = mmap( NULL, 7 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0 );
    usleep( 1000 );

    // Partially deallocated with another mmap.
    char * a6 = mmap( NULL, 7 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0 );
    usleep( 1000 );

    usleep( 3 * 1000 * 1000 );
    munmap( a2, 6 * 4096 );
    munmap( a3, 6 * 4096 );
    munmap( a4 + 4096, 6 * 4096 );
    munmap( a5 + 3 * 4096, 4096 );

    mmap( a6 + 6 * 4096, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0 );
}

int main() {
    foobar();
    return 0;
}
