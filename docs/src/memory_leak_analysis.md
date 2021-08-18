# Case study: Memory leak analysis

If you'd like to follow along this example check out the `simulation`
directory in the root of the profiler's repository where you'll find
the program being analyzed here.

We use the built-in scripting capabilities of the profiler for analysis
here (which is available either through the `script` subcommand of the CLI,
or through the scripting console in the GUI), however you can also use the
built-in GUI to achieve roughly similar results. This example is more about
demonstrating the mindset you need to have when analyzing the data in search
of memory leaks as opposed to a step-by-step guide which you could apply everywhere.

### Step one: let's take a look at the timeline

First let's try to graph all of the allocations:

```rhai,%run
graph()
    .add(allocations())
    .save();
```

It definitely looks like we might have some sort of a memory leak here,
but we can't see much on this graph alone due to all of the noise.

### Step two: let's find all of the obvious memory leaks

Let's try the simplest thing imaginable: graph only those allocations
which were *never* deallocated up until the very end:

```rhai,%run
graph()
    .add("Leaked", allocations().only_leaked())
    .add("Temporary", allocations())
    .save();
```

Aha! We can see an obvious linear growth here! Let's try to split up the leaking part by backtrace:

```rhai,%run
let groups = allocations()
    .only_leaked()
    .group_by_backtrace()
        .sort_by_size();

graph().add(groups).save();
```

Looks like we have a few potential leaks here. First, let's start will
defining a small helper function which will graph *all* of the allocations
coming from that one single backtrace, and also print out that backtrace:

```rhai,%run
fn analyze_group(list) {
    let list_all = allocations().only_matching_backtraces(list);

    graph()
        .add("Leaked", list_all.only_leaked())
        .add("Temporary", list_all)
        .save();

    println("Total: {}", list_all.len());
    println("Leaked: {}", list_all.only_leaked().len());
    println();
    println("Backtrace:");
    println(list_all[0].backtrace().strip());
}
```

#### Group #0

So now let's start with the biggest one:

```rhai,%run
analyze_group(groups[0]);
```

We have a clear-cut memory leak here! Every allocation from this backtrace was leaked.

#### Group #1

Let's try the next one:

```rhai,%run
analyze_group(groups[1]);
```

Looks like while this is *technically* a leak it's just one allocation
made at the very start; we can just ignore this one.

#### Group #2

Let's try yet another one:

```rhai,%run
analyze_group(groups[2]);
```

Now this is interesting. If we only look at the supposedly leaked part it
sure does look like it's an unbounded memory leak which grows lineary with time,
but if we graph *every* allocation from this backtrace we can see that its memory
usage is actually bounded! The longer we would profile this program the more "flat"
the leaked part would get.

So is this a problem? Usually not. If you have something like, say, an LRU cache,
you might see this kind of allocation pattern.

#### Group #3

Let's look at the last group from our original leaked graph:

```rhai,%run
analyze_group(groups[3]);
```

This is the toughest case so far. Do we have a memory leak here on not? Well, it depends.

It could be a case of a bounded leak which hasn't yet reached a saturation point, or it could
be simply a case of only some allocations ending up leaked. We'd either need to profile
for a longer period of time, or analyze the code.


### Step three: we need to go deeper

So is this all? Did we actually find all of the memory leaks? Not necessarily.

What we did was that we only looked at those allocations which were **never** deallocated.
So what about those allocations which *were* deallocated, but *only* at the very end when
the program was shut down? Should we consider those allocations as leaks? Well, probably!

First, let's try to graph the memory usage again, but only including the allocations
which *were* deallocated before the program ended.

```rhai,%run
graph()
    .add(allocations().only_temporary())
    .save();
```

Hmm... there might or might not be a leak here. We need a more powerful filter!

First, let's filter out all of the allocations from the previous section; we've
already analyzed those so we don't want them here to confuse us:

```rhai,%run
let remaining = allocations().only_not_matching_backtraces(groups);
```

And now, we want a list of allocations which weren't deallocated *right until the end*, right?
Well, we can do that!

```rhai,%run
let leaked_until_end = remaining
    .only_not_deallocated_until_at_most(data().runtime() * 0.98);

graph().add(leaked_until_end).save();
```

This indeed looks promising. But let's clean in up a little first.

What's with the peak right at the end? Well, we asked for allocations which were
*not deallocated until 98% of the runtime has elapsed*, so naturally those short
lived allocations from near the end which were also deallocated after that time
will still be included.

Let's get rid of them:

```rhai,%run
let leaked_until_end = remaining
    .only_not_deallocated_until_at_most(data().runtime() * 0.98)
    .only_alive_for_at_least(data().runtime() * 0.02);

graph().add(leaked_until_end).save();
```

Much better!

Now let's graph those by backtrace:

```rhai,%run
let groups = leaked_until_end.group_by_backtrace().sort_by_size();
graph().add(groups).save();
```

Bingo! There *was* something hidden in all of those temporary allocations after all!

Let's define another helper function to help us with our analysis:

```rhai,%run
fn analyze_group(list) {
    let list_all = allocations().only_matching_backtraces(list);
    let list_selected = list_all
        .only_deallocated_after_at_least(data().runtime() * 0.98);

    graph()
        .add("Deallocated after 98%", list_selected)
        .add("Deallocated before 98%", list_all)
        .save();

    println("Total: {}", list_all.len());
    println("Deallocated after 98%: {}", list_selected.len());
    println();
    println("Backtrace:");
    println(list_all[0].backtrace().strip());
}
```

Let's try to use it!

#### Group #0

```rhai,%run
analyze_group(groups[0]);
```

We have a winner! This definitely looks like a leak.

#### Group #1

```rhai,%run
analyze_group(groups[1]);
```

This is `Vec` that, from the look of it, is just growing in size.

It probably *contains* whenever is leaking (and if you read the backtrace
it actualy *does*), but it's not what we're looking for.

In fact, if we look at the original graph most of what we have remaining are
probably cases like this. Let's double-check by filtering out the leak we've already
found and graph everything again:

```rhai,%run
let group = groups
    .ungroup()
    .only_not_matching_backtraces(groups[0])
    .group_by_backtrace();

graph().add(group).save();
```

This does indeed look like all of the long lived allocations here might have been just `Vec`s.

Let's verify that hypothesis:

```rhai,%run
graph()
    .add("Vecs", group.ungroup().only_passing_through_function("raw_vec::finish_grow"))
    .add("Remaining", group.ungroup())
    .save();
```

Indeed we were right!
