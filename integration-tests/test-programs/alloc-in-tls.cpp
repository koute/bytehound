#include <pthread.h>
#include <stdlib.h>
#include <stdio.h>

struct Dummy {
    Dummy() {
        pointer = malloc( 123 );
    }

    ~Dummy() {
        free( pointer );
        free( malloc( 333 ) );
    }

    void * pointer = nullptr;
};

thread_local Dummy dummy;

void * thread_main( void * ) {
    printf( "%p\n", dummy.pointer );
    return nullptr;
}

int main() {
    printf( "%p\n", dummy.pointer );

    pthread_t thread;
    pthread_create( &thread, nullptr, thread_main, nullptr );
    pthread_join( thread, nullptr );

    return 0;
}
