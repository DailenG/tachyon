## Comparison

| Feature | Option A | Option B |
|:--------|:--------:|---------:|
| Startup | **12 ms** | 140 ms |
| Memory  | 30 MB | ~~200 MB~~ 180 MB |
| Escaped pipe | `a \| b` | n/a |

> [!NOTE]
> Numbers were measured on a laptop, see [the methodology][method].

> [!WARNING]
> Option B's numbers include a warm cache.
>
> - It was **not** restarted between runs.

According to the docs[^bench], both options scale linearly. See also [this issue] and
<https://example.com/autolink>.

[method]: https://example.com/methodology "How we measured"
[this issue]: https://github.com/example/project/issues/42

[^bench]: Benchmarks are in `benches/`, run with `cargo bench`.

---

Final recommendation: go with **Option A** unless you need *feature X*.
