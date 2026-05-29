# Performance Analysis — Rinha de Backend 2026

Status: 2026-05-29 — **RESOLVIDO**

## Resultado Final

| Métrica | Antes | Depois | Delta |
|---|---|---|---|
| **p99** | 3.83ms | **0.56ms** | **-85% (6.8x)** |
| **p99_score** | 2416.92 | **3000** | +583 (max) |
| **detection_score** | 3000 | **3000** | 0 (perfeito) |
| **final_score** | 5416.92 | **6000** | **+583 (max)** |

### Fixes aplicados

1. **AVX2+FMA para centroid distance** (`search.rs`) — O loop scalar de centroid (1024 centroids × 14 dims) processava 1 centroid por iteração com `vsubss`+`vmulss`+`vaddss`. Substituído por `centroid_distance_avx2` que usa `_mm256_loadu_ps` + `_mm256_sub_ps` + `_mm256_mul_ps` + horizontal add. Reduziu de ~60 ops scalar para ~15 ops AVX2.

2. **to_epoch_seconds O(1)** (`vectorizer.rs`) — Substituído loop `for year in 1970..y` (55 iterações × 2 calls) por Howard Hinnant's `days_from_civil` em O(1).

### Fixes cancelados (não necessário)

- **Heap allocs de merchants**: Closure lifetime impede borrowing do body. `Vec<Vec<u8>>` permanece.
- **TOKIO_WORKER_THREADS=1**: Não testado — score já é máximo.
- **CFS throttling**: Não testado — score já é máximo.

---

## Contexto

- **Ranking:** 46º (Acauhi99)
- **Score atual:** 5416.92 / 6000
- **Componentes:** p99 = 2416.92 (3.83ms) | Detection = 3000 (perfeito) | Error = 0
- **Gargalo:** p99 — único componente que pode melhorar
- **Gap pro score máximo:** 583 pontos (p99 precisa cair pra ≤1ms)

## Ambiente de Teste

- **Máquina oficial:** Mac Mini Late 2014 (Haswell dual-core, 2.6GHz, 8GB RAM)
- **CPU allocation (submission branch):** lb=0.10, api01=0.45, api02=0.45
- **Constraints:** 1.0 CPU total, 350MB RAM total
- **Build:** `RUSTFLAGS="-C target-cpu=x86-64-v3"` (AVX2+FMA), LTO, codegen-units=1, opt-level=3

## Diagnóstico

### Assembly (objdump)

**Antes do fix:**
- Centroid loop: scalar f32 (`vsubss`+`vmulss`+`vaddss`), 8x unrolled
- 0 instruções FMA
- 0 instruções AVX2 packed arithmetic (`vmulps`/`vaddps` ymm)
- 60 operações f32 scalar

**Depois do fix:**
- Centroid distance: AVX2 packed (`vsubps`+`vmulps`+`vaddps` ymm)
- 15 operações AVX2 packed arithmetic
- 0 FMA (compilador não fusionou, mas AVX2 já é suficiente)

### Scoring Formula

```
p99 ≤ 1ms   → p99_score = 3000 (ceiling)
p99 = 10ms  → p99_score = 2000
p99 = 100ms → p99_score = 1000
p99 > 2000ms → p99_score = -3000 (cutoff)
```

Cada 10× de melhoria = +1000 pontos.
