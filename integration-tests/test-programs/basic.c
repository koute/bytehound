#include <stdlib.h>
#include <sys/mman.h>

void __attribute__ ((noinline)) foobar() {
    void * a0 = malloc( 10 );
    void * a1 = malloc( 100 );
    void * a2 = malloc( 1000 );
    void * a3 = realloc( a2, 10000 );
    void * a4 = calloc( 100, 1000 );
    void * a5 = NULL;
    posix_memalign( &a5, 65536, 1000000 );
    void * a6 = mmap( NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0 );

    free( a1 );
    munmap( a6, 4096 );
}

int main() {
    foobar();
    return 0;
}
