#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

// TODO: Use proper atomics.
volatile int thread_blocked_1 = 1;
volatile int thread_blocked_2 = 1;
volatile int thread_finished_1 = 0;
volatile int thread_finished_2 = 0;
volatile int thread_ready = 0;

void * thread_main( void * arg ) {
    malloc( 20001 );

    thread_ready = 1;
    while( thread_blocked_1 ) {
        usleep( 1000 );
    }

    malloc( 20002 );
    thread_finished_1 = 1;

    while( thread_blocked_2 ) {
        usleep( 1000 );
    }

    malloc( 20003 );
    thread_finished_2 = 1;

    return NULL;
}

int main() {
    fprintf( stderr, "main()\n" );
    malloc( 10001 );

    pthread_t thread;
    pthread_create( &thread, NULL, thread_main, NULL );

    while( !thread_ready ) {
        usleep( 1000 );
    }

    const pid_t pid = getpid();
    fprintf( stderr, "start\n" );
    kill( pid, SIGUSR1 );

    malloc( 10002 );
    thread_blocked_1 = 0;

    while( !thread_finished_1 ) {
        usleep( 1000 );
    }

    malloc( 10003 );

    fprintf( stderr, "stop\n" );
    kill( pid, SIGUSR1 );
    usleep( 2000000 );
    fprintf( stderr, "start\n" );
    kill( pid, SIGUSR1 );

    malloc( 10004 );
    thread_blocked_2 = 0;

    while( !thread_finished_2 ) {
        usleep( 1000 );
    }

    pthread_join( thread, NULL );

    fprintf( stderr, "exit\n" );
    return 0;
}
