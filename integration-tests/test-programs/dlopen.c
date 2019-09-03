#include <dlfcn.h>
#include <unistd.h>

typedef void * (*Callback)();

int main() {
    usleep( 1000 * 1000 );
    void * lib = dlopen( "./dlopen_so", RTLD_NOW );
    Callback cb = (Callback)dlsym( lib, "function" );
    cb();

    return 0;
}
