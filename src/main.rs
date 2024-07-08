use gtk::prelude::Cast;
use gtk::{
    ContainerExt, EditableSignals, Entry, EntryExt, GtkWindowExt, Label, LabelExt, ListBox,
    ListBoxExt, ListBoxRow, WidgetExt, Window, WindowType,
};
use std::cell::RefCell;
use std::io::Read;
use std::io::Write;
use std::net::TcpStream;
use std::rc::Rc;
use std::sync::mpsc;

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;

const MAX_VISIBLE: usize = 10;
const ADDRESS: &str = "127.0.0.1:38451";

// Macro to clone variables for use in closures
macro_rules! clone {
    (@param _) => ( _ );
    (@param $x:ident) => ( $x );
    ($($n:ident),+ => move || $body:expr) => (
        {
            $( let $n = $n.clone(); )+
            move || $body
        }
    );
    ($($n:ident),+ => move |$($p:tt),+| $body:expr) => (
        {
            $( let $n = $n.clone(); )+
            move |$(clone!(@param $p),)+| $body
        }
    );
}

fn main() {
    // Check if the server is already running
    if TcpStream::connect(ADDRESS).is_ok() {
        return;
    }

    // Initialize GTK
    gtk::init().unwrap();

    // Create channels for communication between threads
    let (tx, rx) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
    let (tx2, rx2) = mpsc::channel();

    // Start the server and GUI
    start_server(tx, rx2);
    start_gui(rx, tx2);

    // Start the GTK main loop
    gtk::main();
}

fn start_server(tx: glib::Sender<Vec<String>>, rx2: mpsc::Receiver<String>) {
    std::thread::spawn(move || {
        use std::net::TcpListener;
        let listener = TcpListener::bind(ADDRESS).unwrap();
        let mut listener = listener.incoming();

        loop {
            // Accept incoming connections
            let mut recv_stream = listener.next().unwrap().unwrap();
            let mut send_stream = listener.next().unwrap().unwrap();

            // Read data from the client
            let mut args = String::new();
            recv_stream.read_to_string(&mut args).unwrap();
            if args.is_empty() {
                continue;
            }
            let args = args.lines().map(ToOwned::to_owned).collect();

            // Send data to the GUI thread
            tx.send(args).unwrap();
            let bin = rx2.recv().unwrap();

            // Send response back to the client
            send_stream.write_all(bin.as_bytes()).unwrap();
            send_stream.flush().unwrap();
        }
    });
}

fn start_gui(rx: glib::Receiver<Vec<String>>, tx2: mpsc::Sender<String>) {
    let args: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));

    // Create the main window
    let win = Window::new(WindowType::Toplevel);
    win.set_default_size(400, 300);

    // Create a vertical box to hold the input and list
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let input = Entry::new();
    let fuzzy = ListBox::new();

    // Create a fuzzy matcher
    let matcher = SkimMatcherV2::default();

    // Connect the input change event to update the list
    input.connect_changed(clone!(fuzzy,args => move |ent| {
        let text = ent.get_text();
        let text = text.as_str();
        fuzzy.get_children().iter().for_each(|c| {
            fuzzy.remove(c);
        });
        let args = args.borrow();
        let mut matches: Vec<(i64,&String)> = args
            .iter()
            .filter_map(|imatch|Some((matcher.fuzzy_match(imatch, text)?,imatch)))
            .take(MAX_VISIBLE).collect();

        matches.sort_by_key(|(k,_)|*k);

        matches.into_iter().rev()
            .for_each(|a| {
                fuzzy.add(&Label::new(Some(&a.1)));
            });

        fuzzy.show_all();
        if fuzzy.get_children().is_empty() {
            return;
        }
        fuzzy.select_row(Some(
            &fuzzy.get_children()[0]
                .clone()
                .downcast::<ListBoxRow>()
                .unwrap(),
        ));
    }));

    // Function to hide the window and clear the input and list
    let hide = clone!(win,input,fuzzy =>
    move || {
        win.hide();
        input.set_text("");
        fuzzy.get_children().iter().for_each(|c| {
            fuzzy.remove(c);
        });
    });

    // Connect the window delete event to hide the window
    win.connect_delete_event(clone!(tx2, hide => move |_, _| {
        tx2.send("".into()).unwrap();
        hide();
        gtk::Inhibit(true)
    }));

    // Connect key press events to handle Escape and Return keys
    win.connect_key_press_event(clone!(fuzzy => move |_win, key| match key.get_keyval() {
        gdk::keys::constants::Escape => {
            tx2.send("".into()).unwrap();
            hide();
            gtk::Inhibit(false)
        }
        gdk::keys::constants::Return => {
            if !fuzzy.get_children().is_empty() {
                let bin: Label = fuzzy.get_selected_row().unwrap().get_children()[0]
                    .clone()
                    .downcast()
                    .unwrap();
                tx2.send(bin.get_text().to_string()).unwrap();
            } else {
                tx2.send("".into()).unwrap();
            }
            hide();
            gtk::Inhibit(false)
        }
        _ => gtk::Inhibit(false),
    }));

    // Add the input and list to the vertical box
    vbox.add(&input);
    vbox.add(&fuzzy);
    win.add(&vbox);

    // Attach the receiver to update the list when new data is received
    rx.attach(None, move |new_args| {
        let mut args = args.borrow_mut();
        *args = new_args;
        args.iter().take(MAX_VISIBLE).for_each(|a| {
            fuzzy.add(&Label::new(Some(&a)));
        });
        fuzzy.select_row(Some(
            &fuzzy.get_children()[0]
                .clone()
                .downcast::<ListBoxRow>()
                .unwrap(),
        ));
        win.show_all();
        glib::Continue(true)
    });
}
