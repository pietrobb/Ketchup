use super::super::tests::setup;
use super::*;

fn program(source: &str) -> RuleProgramSource {
    RuleProgramSource {
        variants: Default::default(),
        file_name: "report.star".into(),
        source: source.into(),
        overrides: BTreeMap::new(),
    }
}

/// Sends `request` the way a client connection does and runs one frame.
fn request(app: &mut KetchupApp, sender: &mpsc::SyncSender<Queued>, request: Request) -> Answer {
    let (reply, receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let session = app.live.bridge.as_ref().unwrap().session;
    sender
        .send(Queued {
            session,
            id: 1,
            request,
            connection_closed: false,
            cancelled: Arc::clone(&cancelled),
            reply,
        })
        .unwrap();
    let started = Instant::now();
    app.poll_live_bridge(&egui::Context::default());
    Answer {
        frame: started.elapsed(),
        cancelled,
        receiver,
    }
}

struct Answer {
    frame: Duration,
    cancelled: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Response>,
}

impl Answer {
    fn wait(&self, app: &mut KetchupApp) -> Response {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(response) = self.receiver.recv_timeout(Duration::from_millis(10)) {
                return response;
            }
            assert!(Instant::now() < deadline, "no answer");
            app.poll_live_bridge(&egui::Context::default());
        }
    }
}

fn issues_page(app: &KetchupApp) -> Request {
    Request::ProgramReport {
        expected: app.live_bridge_stamp(),
        section: ReportSection::Issues,
        offset: 0,
        limit: 50,
    }
}

/// Review PRG-1 (capability finding of run 65): a report page whose report
/// must be made again does not hold up the window's frame. 400 boxes that all
/// overlap evaluate at once, but checking their 79 800 pairs takes minutes in
/// a debug build: the frame answers in a moment, the client that gives up stops
/// the worker, and the next page is made and answered while frames go on.
#[test]
fn a_report_page_that_needs_a_slow_check_does_not_hold_up_the_frame() {
    let (mut app, mut bridge) = setup();
    app.new_document();
    app.apply_program_source(program("box('a', (10, 20, 30))"), false)
        .unwrap();
    app.document
        .replace_rule_program_source(program(
            "for i in range(400):\n    box(\"b%d\" % i, (100, 100, 100), at=(0, i * 0.01, 0))\n",
        ))
        .unwrap();
    let (sender, queue) = mpsc::sync_channel(4);
    bridge.queue = queue;
    app.live.bridge = Some(bridge);

    let page = issues_page(&app);
    let slow = request(&mut app, &sender, page);
    assert!(
        slow.frame < Duration::from_secs(5),
        "the frame waited {:?} for the report",
        slow.frame
    );
    assert!(
        slow.receiver.try_recv().is_err(),
        "answered before the check"
    );
    slow.cancelled.store(true, Ordering::Release);
    assert_eq!(
        slow.wait(&mut app).error.as_deref(),
        Some("request_cancelled")
    );

    app.document
        .replace_rule_program_source(program("box('a', (10, 20, 30))"))
        .unwrap();
    let page = issues_page(&app);
    let answer = request(&mut app, &sender, page);
    let response = answer.wait(&mut app);
    assert!(response.ok, "{response:?}");
    assert_eq!(response.result.unwrap()["basis"], "program_evaluation");
}
