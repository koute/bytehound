#include <pthread.h>
#include <stdlib.h>
#include <unistd.h>

void * a0;
void * a1;
void * a2;

void * thread_main_1( void * ) {
    usleep( 100 * 1000 );
    free( a0 );

    a1 = malloc( 1235 );
    a2 = malloc( 1236 );

    return NULL;
}

void * thread_main_2( void * ) {
    usleep( 100 * 1000 );
    free( a1 );

    return NULL;
}

int main() {
    a0 = malloc( 1234 );

    pthread_t thread_1;
    pthread_create( &thread_1, NULL, thread_main_1, NULL );
    pthread_join( thread_1, NULL );

    pthread_t thread_2;
    pthread_create( &thread_2, NULL, thread_main_2, NULL );
    pthread_join( thread_2, NULL );

    usleep( 100 * 1000 );
    free( a2 );

    return 0;
}
