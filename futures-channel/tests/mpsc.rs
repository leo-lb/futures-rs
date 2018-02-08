#[macro_use]
extern crate futures;
extern crate futures_channel;
extern crate futures_executor;

use std::thread;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::prelude::*;
use futures::future::lazy;
use futures_channel::mpsc;
use futures_channel::oneshot;
use futures_executor::current_thread::run;

trait AssertSend: Send {}
impl AssertSend for mpsc::Sender<i32> {}
impl AssertSend for mpsc::Receiver<i32> {}

#[test]
fn send_recv() {
    let (tx, rx) = mpsc::channel::<i32>(16);

    run(|c| {
        c.block_on(tx.send(1)).unwrap();

        assert_eq!(c.block_on(rx.collect()).unwrap(), vec![1]);
    });
}

#[test]
fn send_recv_no_buffer() {
    let (mut tx, mut rx) = mpsc::channel::<i32>(0);

    // Run on a task context
    let f = lazy(move || {
        assert!(tx.poll_complete().unwrap().is_ready());
        assert!(tx.poll_ready().unwrap().is_ready());

        // Send first message
        let res = tx.start_send(1).unwrap();
        assert!(res.is_ok());
        assert!(tx.poll_ready().unwrap().is_not_ready());

        // Send second message
        let res = tx.start_send(2).unwrap();
        assert!(res.is_err());

        // Take the value
        assert_eq!(rx.poll().unwrap(), Async::Ready(Some(1)));
        assert!(tx.poll_ready().unwrap().is_ready());

        let res = tx.start_send(2).unwrap();
        assert!(res.is_ok());
        assert!(tx.poll_ready().unwrap().is_not_ready());

        // Take the value
        assert_eq!(rx.poll().unwrap(), Async::Ready(Some(2)));
        assert!(tx.poll_ready().unwrap().is_ready());

        Ok::<(), ()>(())
    });
    run(|c| c.block_on(f)).unwrap();
}

#[test]
fn send_shared_recv() {
    let (tx1, rx) = mpsc::channel::<i32>(16);
    let tx2 = tx1.clone();

    run(|c| {
        c.block_on(tx1.send(1)).unwrap();
        let (item, rx) = c.block_on(rx.into_future()).ok().unwrap();
        assert_eq!(item, Some(1));

        c.block_on(tx2.send(2)).unwrap();
        let item = c.block_on(rx.into_future()).ok().unwrap().0;
        assert_eq!(item, Some(2));
    });
}

#[test]
fn send_recv_threads() {
    let (tx, rx) = mpsc::channel::<i32>(16);

    let t = thread::spawn(move|| {
        run(|c| c.block_on(tx.send(1)))
    });

    assert_eq!(run(|c| c.block_on(rx.take(1).collect())).unwrap(), vec![1]);

    t.join().unwrap().unwrap();
}

#[test]
fn send_recv_threads_no_capacity() {
    let (tx, rx) = mpsc::channel::<i32>(0);

    let (readytx, readyrx) = mpsc::channel::<()>(2);
    let t = thread::spawn(move|| {
        let readytx = readytx.sink_map_err(|_| panic!());
        run(|c| {
            let (a, b) = c.block_on(tx.send(1).join(readytx.send(()))).unwrap();
            c.block_on(a.send(2).join(b.send(()))).unwrap();
        })
    });

    run(|c| {
        let readyrx = c.block_on(readyrx.into_future()).ok().unwrap().1;
        let (item, rx) = c.block_on(rx.into_future()).ok().unwrap();
        assert_eq!(item, Some(1));
        drop(c.block_on(readyrx.into_future()).ok().unwrap());
        let item = c.block_on(rx.into_future()).ok().unwrap().0;
        assert_eq!(item, Some(2));
    });

    t.join().unwrap();
}

#[test]
fn recv_close_gets_none() {
    let (mut tx, mut rx) = mpsc::channel::<i32>(10);

    // Run on a task context
    let f = lazy(move || {
        rx.close();

        assert_eq!(rx.poll(), Ok(Async::Ready(None)));
        assert!(tx.poll_ready().is_err());

        drop(tx);

        Ok::<(), ()>(())
    });

    run(|c| c.block_on(f)).unwrap();
}

#[test]
fn tx_close_gets_none() {
    let (_, mut rx) = mpsc::channel::<i32>(10);

    // Run on a task context
    let f = lazy(move || {
        assert_eq!(rx.poll(), Ok(Async::Ready(None)));
        assert_eq!(rx.poll(), Ok(Async::Ready(None)));

        Ok::<(), ()>(())
    });

    run(|c| c.block_on(f)).unwrap();
}

// #[test]
// fn spawn_sends_items() {
//     let core = local_executor::Core::new();
//     let stream = unfold(0, |i| Some(ok::<_,u8>((i, i + 1))));
//     let rx = mpsc::spawn(stream, &core, 1);
//     assert_eq!(core.run(rx.take(4).collect()).unwrap(),
//                [0, 1, 2, 3]);
// }

// #[test]
// fn spawn_kill_dead_stream() {
//     use std::thread;
//     use std::time::Duration;
//     use futures::future::Either;
//     use futures::sync::oneshot;
//
//     // a stream which never returns anything (maybe a remote end isn't
//     // responding), but dropping it leads to observable side effects
//     // (like closing connections, releasing limited resources, ...)
//     #[derive(Debug)]
//     struct Dead {
//         // when dropped you should get Err(oneshot::Canceled) on the
//         // receiving end
//         done: oneshot::Sender<()>,
//     }
//     impl Stream for Dead {
//         type Item = ();
//         type Error = ();
//
//         fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
//             Ok(Async::Pending)
//         }
//     }
//
//     // need to implement a timeout for the test, as it would hang
//     // forever right now
//     let (timeout_tx, timeout_rx) = oneshot::channel();
//     thread::spawn(move || {
//         thread::sleep(Duration::from_millis(1000));
//         let _ = timeout_tx.send(());
//     });
//
//     let core = local_executor::Core::new();
//     let (done_tx, done_rx) = oneshot::channel();
//     let stream = Dead{done: done_tx};
//     let rx = mpsc::spawn(stream, &core, 1);
//     let res = core.run(
//         Ok::<_, ()>(())
//         .into_future()
//         .then(move |_| {
//             // now drop the spawned stream: maybe some timeout exceeded,
//             // or some connection on this end was closed by the remote
//             // end.
//             drop(rx);
//             // and wait for the spawned stream to release its resources
//             done_rx
//         })
//         .select2(timeout_rx)
//     );
//     match res {
//         Err(Either::A((oneshot::Canceled, _))) => (),
//         _ => {
//             panic!("dead stream wasn't canceled");
//         },
//     }
// }

#[test]
fn stress_shared_unbounded() {
    const AMT: u32 = 10000;
    const NTHREADS: u32 = 8;
    let (tx, rx) = mpsc::unbounded::<i32>();

    let t = thread::spawn(move|| {
        run(|c| {
            let result = c.block_on(rx.collect()).unwrap();
            assert_eq!(result.len(), (AMT * NTHREADS) as usize);
            for item in result {
                assert_eq!(item, 1);
            }
        });
    });

    for _ in 0..NTHREADS {
        let tx = tx.clone();

        thread::spawn(move|| {
            for _ in 0..AMT {
                tx.unbounded_send(1).unwrap();
            }
        });
    }

    drop(tx);

    t.join().ok().unwrap();
}

#[test]
fn stress_shared_bounded_hard() {
    const AMT: u32 = 10000;
    const NTHREADS: u32 = 8;
    let (tx, rx) = mpsc::channel::<i32>(0);

    let t = thread::spawn(move|| {
        run(|c| {
            let result = c.block_on(rx.collect()).unwrap();
            assert_eq!(result.len(), (AMT * NTHREADS) as usize);
            for item in result {
                assert_eq!(item, 1);
            }
        });
    });

    for _ in 0..NTHREADS {
        let mut tx = tx.clone();

        thread::spawn(move || {
            run(|c| {
                for _ in 0..AMT {
                    tx = c.block_on(tx.send(1)).unwrap();
                }
            });
        });
    }

    drop(tx);

    t.join().unwrap();
}

#[test]
fn stress_receiver_multi_task_bounded_hard() {
    const AMT: usize = 10_000;
    const NTHREADS: u32 = 2;

    let (mut tx, rx) = mpsc::channel::<usize>(0);
    let rx = Arc::new(Mutex::new(Some(rx)));
    let n = Arc::new(AtomicUsize::new(0));

    let mut th = vec![];

    for _ in 0..NTHREADS {
        let rx = rx.clone();
        let n = n.clone();

        let t = thread::spawn(move || {
            let mut i = 0;

            run(|c| {
                loop {
                    i += 1;
                    let mut lock = rx.lock().ok().unwrap();

                    let mut rx = match lock.take() {
                        Some(rx) => rx,
                        None => break,
                    };
                    if i % 5 == 0 {
                        let (item, rest) = c.block_on(rx.into_future()).ok().unwrap();

                        if item.is_none() {
                            break;
                        }

                        n.fetch_add(1, Ordering::Relaxed);
                        *lock = Some(rest);
                    } else {
                        // Just poll
                        let n = n.clone();
                        let f = lazy(move || {
                            let r = match rx.poll().unwrap() {
                                Async::Ready(Some(_)) => {
                                    n.fetch_add(1, Ordering::Relaxed);
                                    *lock = Some(rx);
                                    false
                                }
                                Async::Ready(None) => {
                                    true
                                }
                                Async::Pending => {
                                    *lock = Some(rx);
                                    false
                                }
                            };

                            Ok::<bool, ()>(r)
                        });

                        if c.block_on(f).unwrap() {
                            break;
                        }
                    }
                }
            });
        });

        th.push(t);
    }

    run(move |c| {
        for i in 0..AMT {
            tx = c.block_on(tx.send(i)).unwrap();
        }
    });

    for t in th {
        t.join().unwrap();
    }

    assert_eq!(AMT, n.load(Ordering::Relaxed));
}

/// Stress test that receiver properly receives all the messages
/// after sender dropped.
#[test]
fn stress_drop_sender() {
    fn list() -> Box<Stream<Item=i32, Error=u32>> {
        let (tx, rx) = mpsc::channel(1);
        let f = tx.send(Ok(1))
          .and_then(|tx| tx.send(Ok(2)))
          .and_then(|tx| tx.send(Ok(3)));
        thread::spawn(move || {
            run(|c| c.block_on(f).unwrap());
        });
        Box::new(rx.then(|r| r.unwrap()))
    }

    run(|c| {
        for _ in 0..10000 {
            assert_eq!(c.block_on(list().collect()).unwrap(), vec![1, 2, 3]);
        }
    });
}

/// Stress test that after receiver dropped,
/// no messages are lost.
fn stress_close_receiver_iter() {
    let (tx, rx) = mpsc::unbounded();
    let (unwritten_tx, unwritten_rx) = std::sync::mpsc::channel();
    let th = thread::spawn(move || {
        for i in 1.. {
            if let Err(_) = tx.unbounded_send(i) {
                unwritten_tx.send(i).expect("unwritten_tx");
                return;
            }
        }
    });

    // Read one message to make sure thread effectively started
    run(|c| {
        let (item, mut rx) = c.block_on(rx.into_future()).ok().unwrap();
        assert_eq!(Some(1), item);

        rx.close();

        for i in 2.. {
            let (item, r) = c.block_on(rx.into_future()).ok().unwrap();
            rx = r;
            match item {
                Some(r) => assert!(i == r),
                None => {
                    let unwritten = unwritten_rx.recv().expect("unwritten_rx");
                    assert_eq!(unwritten, i);
                    th.join().unwrap();
                    return;
                }
            }
        }
    });
}

#[test]
fn stress_close_receiver() {
    for _ in 0..10000 {
        stress_close_receiver_iter();
    }
}

/// Tests that after `poll_ready` indicates capacity a channel can always send without waiting.
#[test]
fn stress_poll_ready() {
    // A task which checks channel capacity using poll_ready, and pushes items onto the channel when
    // ready.
    struct SenderTask {
        sender: mpsc::Sender<u32>,
        count: u32,
    }
    impl Future for SenderTask {
        type Item = ();
        type Error = ();

        fn poll(&mut self) -> Poll<(), ()> {
            // In a loop, check if the channel is ready. If so, push an item onto the channel
            // (asserting that it doesn't attempt to block).
            while self.count > 0 {
                try_ready!(self.sender.poll_ready().map_err(|_| ()));
                assert!(self.sender.start_send(self.count).unwrap().is_ok());
                self.count -= 1;
            }
            Ok(Async::Ready(()))
        }
    }

    const AMT: u32 = 1000;
    const NTHREADS: u32 = 8;

    /// Run a stress test using the specified channel capacity.
    fn stress(capacity: usize) {
        let (tx, rx) = mpsc::channel(capacity);
        let mut threads = Vec::new();
        for _ in 0..NTHREADS {
            let sender = tx.clone();
            threads.push(thread::spawn(move || {
                run(|c| {
                    c.block_on(SenderTask {
                        sender: sender,
                        count: AMT,
                    })
                })
            }));
        }
        drop(tx);

        let result = run(|c| c.block_on(rx.collect())).unwrap();
        assert_eq!(result.len() as u32, AMT * NTHREADS);

        for thread in threads {
            thread.join().unwrap().unwrap();
        }
    }

    stress(0);
    stress(1);
    stress(8);
    stress(16);
}

#[test]
fn try_send_1() {
    const N: usize = 3000;
    let (mut tx, rx) = mpsc::channel(0);

    let t = thread::spawn(move || {
        for i in 0..N {
            loop {
                if tx.try_send(i).is_ok() {
                    break
                }
            }
        }
    });

    let result = run(|c| c.block_on(rx.collect())).unwrap();
    for (i, j) in result.into_iter().enumerate() {
        assert_eq!(i, j);
    }

    t.join().unwrap();
}

#[test]
fn try_send_2() {
    let (mut tx, rx) = mpsc::channel(0);

    tx.try_send("hello").unwrap();

    let (readytx, readyrx) = oneshot::channel::<()>();

    let th = thread::spawn(|| {
        run(|c| {
            c.block_on(lazy(|| {
                assert!(tx.start_send("fail").unwrap().is_err());
                Ok::<_, ()>(())
            })).unwrap();

            drop(readytx);
            c.block_on(tx.send("goodbye")).unwrap();
        })
    });

    run(|c| {
        drop(c.block_on(readyrx));
        let (item, rx) = c.block_on(rx.into_future()).ok().unwrap();
        assert_eq!(item, Some("hello"));
        let (item, rx) = c.block_on(rx.into_future()).ok().unwrap();
        assert_eq!(item, Some("goodbye"));
        let item = c.block_on(rx.into_future()).ok().unwrap().0;
        assert_eq!(item, None);
    });

    th.join().unwrap();
}

#[test]
fn try_send_fail() {
    let (mut tx, rx) = mpsc::channel(0);

    tx.try_send("hello").unwrap();

    // This should fail
    assert!(tx.try_send("fail").is_err());

    run(|c| {
        let (item, rx) = c.block_on(rx.into_future()).ok().unwrap();
        assert_eq!(item, Some("hello"));

        tx.try_send("goodbye").unwrap();
        drop(tx);

        let (item, rx) = c.block_on(rx.into_future()).ok().unwrap();
        assert_eq!(item, Some("goodbye"));
        let item = c.block_on(rx.into_future()).ok().unwrap().0;
        assert_eq!(item, None);
    });
}
