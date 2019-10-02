#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <setjmp.h>

int catch_1 = 0;
int catch_2 = 0;

int f1 = 0;
int f2 = 0;
int f3a = 0;
int f3b = 0;
int f4 = 0;
int f5a = 0;
int f5b = 0;

jmp_buf buf_a;
jmp_buf buf_b;

void __attribute__ ((noinline)) foobar_0() {
    malloc( 123456 );
}

void __attribute__ ((noinline)) foobar_1() {
    foobar_0();

    printf( ">> before throw\n" );
    longjmp( buf_a, 1 );
    f1 = 1;
}

void __attribute__ ((noinline)) foobar_2() {
    foobar_1();
    f2 = 1;
}

void __attribute__ ((noinline)) foobar_3() {
    printf( ">> before try\n" );
    if( setjmp( buf_a ) == 0 ) {
        foobar_2();
        f3a = 1;
    } else {
        catch_1 = 1;
        printf( ">> inside catch\n" );
        malloc( 123457 );
        longjmp( buf_b, 1 );
    }
    f3b = 1;
}

void __attribute__ ((noinline)) foobar_4() {
    foobar_3();
    f4 = 1;
}

void __attribute__ ((noinline)) foobar_5() {
    if( setjmp( buf_b ) == 0 ) {
        foobar_4();
        f5a = 1;
    } else {
        catch_2 = 1;
        malloc( 123458 );
    }
    f5b = 1;
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
