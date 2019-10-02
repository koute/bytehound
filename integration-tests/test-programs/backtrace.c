#include <stdlib.h>
#include <execinfo.h>

void __attribute__ ((noinline)) foo() {
    malloc( 123456 );

    void * buffer[ 32 ];
    const int count = backtrace( buffer, 32 );
    if( count == 0 ) {
        exit( 1 );
    }
    char ** symbols = backtrace_symbols( buffer, count );
    free( symbols );
}

void __attribute__ ((noinline)) bar() {
    foo();

    void * buffer[ 32 ];
    const int count = backtrace( buffer, 32 );
    if( count == 0 ) {
        exit( 1 );
    }
    char ** symbols = backtrace_symbols( buffer, count );
    free( symbols );
}

int main() {
    bar();
    return 0;
}
