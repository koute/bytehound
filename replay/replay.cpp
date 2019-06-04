#ifndef _GNU_SOURCE
#define _GNU_SOURCE 1
#endif

#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <malloc.h>
#include <pthread.h>

#define OP_END     0
#define OP_ALLOC   1
#define OP_FREE    2
#define OP_REALLOC 3
#define OP_GO_DOWN 4
#define OP_GO_UP   5

typedef struct Data Data;
typedef struct Op Op;

struct Op {
    uint64_t kind;
    union {
        struct {
            uint64_t slot;
            uint64_t timestamp;
            uint64_t size;
        } alloc;
        struct {
            uint64_t slot;
            uint64_t timestamp;
        } free;
        struct {
            uint64_t slot;
            uint64_t timestamp;
            uint64_t size;
        } realloc;
        struct {
            uint64_t frame;
        } go_down;
    };
};

struct Data {
    uint64_t slot_count;
    Op operations[];
};

void * mmap_file(const char * path) {
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        perror("open failed");
        exit(1);
    }

    struct stat sb;
    fstat(fd, &sb);
    void * result = mmap(NULL, sb.st_size, PROT_READ, MAP_SHARED, fd, 0);
    if (result == MAP_FAILED) {
        perror("mmap failed");
        exit(1);
    }

    return result;
}

void * mmap_anonymous(size_t size) {
    void * result = mmap(NULL, (size + 4096) & ~4096, PROT_READ | PROT_WRITE, MAP_ANONYMOUS | MAP_PRIVATE, -1, 0);
    if (result == MAP_FAILED) {
        perror("anonymous mmap failed");
        exit(1);
    }
    return result;
}

void default_set_marker(uint32_t marker) {
}

void default_override_next_timestamp(uint64_t timestamp) {
}

struct State {
    size_t i;
    Data * data;
    void ** slots;
    size_t count;
};

typedef void (*override_next_timestamp_t)(uint64_t timestamp);
typedef void (*set_marker_t)(uint32_t marker);
typedef void (*frame_t)(State&);

static size_t count = 0;
static set_marker_t set_marker;
static override_next_timestamp_t override_next_timestamp;

extern frame_t FRAMES[];
extern size_t FRAME_COUNT;

void __attribute__ ((noinline)) frame_default(State&);

static inline void __attribute__ ((always_inline)) go_down(State& state, uint64_t frame) {
    frame_t cb = frame_default;
    if (frame < FRAME_COUNT) {
        cb = FRAMES[frame];
    }

    cb(state);
}

static inline void __attribute__ ((always_inline)) run(State& state) {
    for (;;) {
        const Op * op = &state.data->operations[state.i];
        if (op->kind == OP_END) {
            return;
        }
        state.i++;

        switch (op->kind) {
            case OP_ALLOC:
                state.count++;
                if (state.slots[op->alloc.slot] != NULL) {
                    abort();
                }
                override_next_timestamp(op->alloc.timestamp);
                state.slots[op->alloc.slot] = malloc(op->alloc.size);
                break;
            case OP_FREE:
                override_next_timestamp(op->free.timestamp);
                free(state.slots[op->free.slot]);
                state.slots[op->free.slot] = NULL;
                break;
            case OP_REALLOC:
                state.count++;
                override_next_timestamp(op->realloc.timestamp);
                state.slots[op->realloc.slot] = realloc(state.slots[op->realloc.slot], op->realloc.size);
                break;
            case OP_GO_DOWN:
                go_down(state, op->go_down.frame);
                break;
            case OP_GO_UP:
                return;
            default:
                abort();
        }
    }
}

template <int N>
void __attribute__ ((noinline)) frame_n(State& state) {
    run(state);
    asm("");
}

void __attribute__ ((noinline)) frame_default(State& state) {
    run(state);
    asm("");
}

State run_for_data(Data * data) {
    void ** slots = (void **)mmap_anonymous(data->slot_count * sizeof(void *));

    State state;
    state.data = data;
    state.slots = slots;
    state.i = 0;
    state.count = 0;
    run(state);

    return state;
}

#include "generated.inc"

void * benchmark_thread_main(void * data_ptr) {
    Data * data = (Data *)data_ptr;
    run_for_data(data);
    return nullptr;
}

int main(int argc, char * argv[]) {
    bool benchmark_mode = false;
    bool args_are_valid = true;
    const char * input = nullptr;

    for (int i = 1; i < argc; ++i) {
        char * arg = argv[i];
        if (!strcmp(arg, "--benchmark")) {
            benchmark_mode = true;
        } else {
            if (input != nullptr) {
                args_are_valid = false;
                break;
            }

            input = arg;
        }
    }

    args_are_valid = args_are_valid && input != nullptr;

    if (!args_are_valid) {
        fprintf(stderr, "syntax: replay [--benchmark] <replay.dat>\n");
        return 1;
    }

    if (!benchmark_mode) {
        set_marker = (set_marker_t)dlsym(RTLD_DEFAULT, "memory_profiler_set_marker");
        override_next_timestamp = (override_next_timestamp_t)dlsym(RTLD_DEFAULT, "memory_profiler_override_next_timestamp");
    } else {
        puts("Running in benchmark mode...");
    }

    if (!set_marker) {
        set_marker = default_set_marker;
    }

    if (!override_next_timestamp) {
        override_next_timestamp = default_override_next_timestamp;
    }

    Data * data = (Data *)mmap_file(input);
    if (!benchmark_mode) {
        State state = run_for_data(data);
        printf("total allocations: %i\n", state.count);
    } else {
        pthread_t threads[3];
        for (int i = 0; i < 3; ++i) {
            pthread_create(&threads[i], nullptr, benchmark_thread_main, (void *)data);
        }
        for (int i = 0; i < 3; ++i) {
            pthread_join(threads[i], nullptr);
        }
    }

    printf("free: %i\n", mallinfo().fordblks);
    printf("fast free: %i\n", mallinfo().fsmblks);
    printf("fast free blocks: %i\n", mallinfo().smblks);

    return 0;
}
