#include <unistd.h>
#include <stdlib.h>
#include <signal.h>

volatile int counter = 10001;
volatile int running = 1;
volatile int has_to_die = 0;

static void signal_handler_sigusr1( int signal ) {
    counter++;
}

static void signal_handler_sigusr2( int signal ) {
    has_to_die = 1;
}

static void signal_handler_sigint( int signal ) {
    running = 0;
}

int main() {
    int last_counter = counter;
    malloc( last_counter );

    signal( SIGUSR1, signal_handler_sigusr1 );
    signal( SIGUSR2, signal_handler_sigusr2 );
    signal( SIGINT, signal_handler_sigint );

    for( ;; ) {
        usleep( 1000 );
        if( counter != last_counter ) {
            last_counter = counter;
            malloc( last_counter );
        }

        if( !running ) {
            return 0;
        }

        if( has_to_die ) {
            kill( getpid(), SIGKILL );
            for( ;; ) { usleep( 1000 ); }
        }
    }
}
