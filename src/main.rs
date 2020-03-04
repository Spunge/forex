
extern crate jack;

use std::sync::{Arc, Mutex};
use gilrs::{Gilrs, Event};
use std::time::SystemTime;
use std::io;

/*
 * Midi message representation
 */
#[derive(Debug, Copy, Clone)]
struct Message {
    time: SystemTime,
    channel: u8,
    note: u8,
    velocity: u8,
}

impl Message {
    fn new(time: SystemTime, channel: u8, tom_id: u32, velocity: f32) -> Self {
        Self {
            time,
            note: tom_id as u8 + 36,
            channel,
            // Turn -0.1259 & 0.827 to 0 ... 127
            velocity: ((1.0 - velocity.abs()) * 127.0) as u8,
        }
    }
}

/*
 * Tom will keep track of hits
 */
#[derive(Debug)]
struct Tom {
    id: u32,
    hit: Option<SystemTime>,
    velocity: Option<f32>,
}

impl Tom {
    fn new(id: u32) -> Self {
        Self { id, hit: None, velocity: None }
    }

    // Remember if we're hit (so we can output next velocity event)
    fn hit(&mut self, time: SystemTime) {
        self.hit = Some(time);
    }
    fn release(&mut self, time: SystemTime) -> Option<Message> {
        if let Some(velocity) = self.velocity {
            // 0x80 = note off on channel 1; 0x81 is note off on channel 2
            Some(Message::new(time, 0x80, self.id, velocity))
        } else {
            None
        }
    }
    fn record_velocity(&mut self, velocity: f32, time: SystemTime) -> Option<Message> {
        self.velocity = Some(velocity);

        // If we were hit, output note on message
        if let Some(_time_of_hit) = self.hit {
            self.hit = None;
            // 0x90 = note on on channel 1; 0x91 is note on on channel 2
            Some(Message::new(time, 0x90, self.id, velocity))
        } else {
            None
        }
    }
}

/*
 * Drum will hold toms
 */
struct Drums {
    toms: [Tom; 6],
}

impl Drums {
    fn process_event(&mut self, event: gilrs::EventType, time: SystemTime) -> Option<Message> {
        match event {
            gilrs::EventType::ButtonPressed(_, code) => {
                let index = code.into_u32() - 65824;
                self.toms[index as usize].hit(time);
                None
            },
            gilrs::EventType::ButtonReleased(_, code) => {
                let index = code.into_u32() - 65824;
                self.toms[index as usize].release(time)
            },
            gilrs::EventType::AxisChanged(_, velocity, code) => {
                let weird_index = code.into_u32() - 196608;
                let index = match weird_index {
                    0 | 1 | 4 => weird_index,
                    3 => 5,
                    5 => 3,
                    6 => 2,
                    _ => 0,
                };
                self.toms[index as usize].record_velocity(velocity, time)
            }
            _ => None,
        }
    }
}

/*
 * This is our jack process handler
 */
struct Processor {
    // Gilrs contains a raw pointer to a udev_monitor, wrap it for thread safety
    gilrs: Arc<Mutex<Gilrs>>,
    drums: Drums,
    output: jack::Port<jack::MidiOut>,
}

impl Processor {
    fn new(client: &jack::Client) -> Self {
        let drums = Drums {
            toms: [Tom::new(0), Tom::new(1), Tom::new(2), Tom::new(3), Tom::new(4), Tom::new(5)],
        };

        let gilrs = Arc::new(Mutex::new(Gilrs::new().unwrap()));
        
        // Iterate over all connected gamepads
        for (_id, gamepad) in gilrs.lock().unwrap().gamepads() {
            println!("{} is {:?}", gamepad.id(), gamepad.name());
        }

        let output = client.register_port("output", jack::MidiOut::default()).unwrap();

        Self { gilrs, drums, output }
    }
}

// As we totally really did wrap all our thread unsafe stuff in processor, mark it as thread safe
unsafe impl Send for Processor {}
unsafe impl Sync for Processor {}

impl jack::ProcessHandler for Processor {
    fn process(&mut self, _: &jack::Client, process_scope: &jack::ProcessScope) -> jack::Control {

        let mut writer = self.output.writer(process_scope);

        while let Some(Event { id: _, event, time }) = self.gilrs.lock().unwrap().next_event() {
            if let Some(message) = self.drums.process_event(event, time) {
                // Get some information about our current cycle & message
                let message_usecs_ago = SystemTime::now().duration_since(message.time).unwrap().as_micros();
                let period_usecs = process_scope.cycle_times().unwrap().period_usecs;

                // Calculate when message occurred, send out midi
                let message_frames_ago = ((message_usecs_ago as f32 / period_usecs as f32) * process_scope.n_frames() as f32) as u32;
                let message_frame = process_scope.n_frames() - message_frames_ago;

                // TODO - System time can go all over the place, so this can fail..
                writer.write(&jack::RawMidi { time: message_frame, bytes: &[ message.channel, message.note, message.velocity ] });
            }
        }

        jack::Control::Continue
    }
}

fn main() {
    let (client, _status) = jack::Client::new("Forex", jack::ClientOptions::NO_START_SERVER).unwrap();

    // Add processhandler & start client
    let processor = Processor::new(&client);
    let _active_client = client.activate_async((), processor);

    // Wait for user to input string (to not exit)
    let mut user_input = String::new();
    io::stdin().read_line(&mut user_input).ok();
}

