#include <stdlib.h>

void __attribute__ ((noinline)) foobar() {
    void * a0 = malloc( 10 );
    void * a1 = malloc( 100 );
    void * a2 = malloc( 1000 );
    void * a3 = realloc( a2, 10000 );
    void * a4 = calloc( 100, 1000 );

    free( a1 );
}

int main() {
    foobar();
    return 0;
}
