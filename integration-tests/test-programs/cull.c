#include <stdlib.h>
#include <unistd.h>

static size_t counter = 1234;

void __attribute__ ((noinline)) foobar( useconds_t sleep_for ) {
    void * a0 = malloc( counter );
    counter += 1;

    if( sleep_for != 0 ) {
        usleep( sleep_for );
        free( a0 );
    }
}

const static useconds_t SLEEP_FOR[3] = { 1, 1000000, 0 };

int main() {
    for( int i = 0; i < 3; ++i ) {
        foobar( SLEEP_FOR[i] );
    }

    void * a0 = malloc( 2000 );
    realloc( a0, 3000 );

    usleep( 500000 );
    return 0;
}
