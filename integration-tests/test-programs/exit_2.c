#include <stdlib.h>
#include <unistd.h>

int main() {
    malloc( 12001 );
    _exit( 0 );
}
