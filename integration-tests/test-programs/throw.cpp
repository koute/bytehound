#include <stdio.h>
#include <string.h>
#include <stdlib.h>

bool catch_1 = false;
bool catch_2 = false;

bool f1 = false;
bool f2 = false;
bool f3a = false;
bool f3b = false;
bool f4 = false;
bool f5a = false;
bool f5b = false;

extern "C" {

void __attribute__ ((noinline)) foobar_0() {
    malloc( 123456 );
}

void __attribute__ ((noinline)) foobar_1() {
    foobar_0();

    printf( ">> before throw\n" );
    throw "dummy";
    f1 = true;
}

void __attribute__ ((noinline)) foobar_2() {
    foobar_1();
    f2 = true;
}

void __attribute__ ((noinline)) foobar_3() {
    printf( ">> before try\n" );
    try {
        foobar_2();
        f3a = true;
    } catch (...) {
        catch_1 = true;
        printf( ">> inside catch\n" );
        malloc( 123457 );
        throw;
    }
    f3b = true;
}

void __attribute__ ((noinline)) foobar_4() {
    foobar_3();
    f4 = true;
}

void __attribute__ ((noinline)) foobar_5() {
    try {
        foobar_4();
        f5a = true;
    } catch (...) {
        catch_2 = true;
        malloc( 123458 );
    }
    f5b = true;
}

}

int main() {
    printf( ">> start\n" );
    foobar_5();

    if( catch_1 && catch_2 && !f1 && !f2 && !f3a && !f3b && !f4 && !f5a && f5b ) {
        malloc( 123459 );
        return 0;
    }

    abort();
}
