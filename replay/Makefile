all: replay

replay: replay.cpp generated.inc
	c++ replay.cpp -O1 -fno-tree-tail-merge -fno-inline -fasynchronous-unwind-tables -o replay -ldl -pthread

generated.inc: generate.sh
	./generate.sh
