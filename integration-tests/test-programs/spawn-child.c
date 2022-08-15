#include <stdlib.h>
#include <unistd.h>
#include <sys/wait.h>

int main() {
    usleep( 100000 );
    malloc( 10001 );

    pid_t pid = fork();
    if( pid == 0 ) {
        // Child
        if (execl("./basic", "./basic", NULL) == -1) {
            return 1;
        }
        return 0;
    }

    waitpid(pid, NULL, 0);
    malloc( 10003 );

    return 0;
}
