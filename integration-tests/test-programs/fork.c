#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

// TODO: Use proper atomics.
volatile int thread_blocked = 1;
volatile int thread_finished = 0;
volatile int thread_ready = 0;

void * thread_main( void * arg ) {
    malloc( 20001 );

    thread_ready = 1;
    while( thread_blocked ) {
        usleep( 1000 );
    }

    malloc( 20002 );
    thread_finished = 1;

    return NULL;
}

int main() {
    usleep( 100000 );
    malloc( 10001 );

    pthread_t thread;
    pthread_create( &thread, NULL, thread_main, NULL );

    while( !thread_ready ) {
        usleep( 1000 );
    }

    malloc( 10002 );

    pid_t pid = fork();
    if( pid == 0 ) {
        // Child
        malloc( 30000 );

        pthread_t thread_2;
        pthread_create( &thread_2, NULL, thread_main, NULL );
        thread_blocked = 0;
        while( !thread_finished ) {
            usleep( 1000 );
        }

        malloc( 30001 );
        return 0;
    }

    thread_blocked = 0;
    while( !thread_finished ) {
        usleep( 1000 );
    }

    malloc( 10003 );
    return 0;
}
