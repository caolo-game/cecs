use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::hint::black_box;

use cecs::prelude::*;

macro_rules! components {
    ($ ($x: ident),*) => {

        $(
        #[derive(Clone)]
        #[allow(unused)]
        struct $x (pub [u8; 32]);
        )*

        fn add_entity<'a>(cmd: &'a mut Commands) -> &'a mut EntityCommands {
            let mut cmd = cmd.spawn();
            $(
                if fastrand::bool() {
                    cmd = cmd.insert(
                        $x([42; 32])
                    );
                }
            )*
            cmd
        }

    };
}

components!(
    C1, C2, C3, C4, C5, C6, C7, C8, C9, C10, C11, C12, C13, C14, C15, C16, C17, C18, C19, C20, C21,
    C22, C23, C24, C25, C26, C27, C28, C29, C30, C31, C32, C33, C34, C35, C36, C37, C38, C39, C40,
    C41, C42, C43, C44, C45, C46, C47, C48, C49, C50
);

#[derive(Clone, Copy)]
struct TestComponent(pub u64);

fn benchmark_iter(c: &mut Criterion) {
    fastrand::seed(0xdeadbeef);
    let mut group = c.benchmark_group("query");

    for n in (10..=14).step_by(2) {
        let n = 1 << n;
        let mut world = World::new(n);

        world
            .run_system(|mut cmd: Commands| {
                for _ in 0..n {
                    add_entity(&mut cmd).insert(TestComponent(0));
                }
            })
            .unwrap();

        group.bench_with_input(BenchmarkId::new("mutate-single-serial", n), &n, |b, _n| {
            b.iter(|| {
                world
                    .run_system(|mut q: Query<&mut TestComponent>| {
                        for c in q.iter_mut() {
                            c.0 = 0xBEEF;
                        }
                    })
                    .unwrap();

                black_box(&world);
            });
        });

        group.bench_with_input(
            BenchmarkId::new("mutate-single-parallel", n),
            &n,
            |b, _n| {
                b.iter(|| {
                    world
                        .run_system(|mut q: Query<&mut TestComponent>| {
                            q.par_for_each_mut(|c| {
                                c.0 = 0xBEEF;
                            });
                        })
                        .unwrap();

                    black_box(&world);
                });
            },
        );
    }
}

criterion_group!(query_benches, benchmark_iter);
criterion_main!(query_benches);
