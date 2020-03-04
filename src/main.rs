
extern crate jack;

use std::sync::{Arc, Mutex};
use gilrs::{Gilrs, Event};
use std::time::SystemTime;
use std::io;

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
            velocity: (velocity * 128.0) as u8 / 2,
        }
    }
}

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

    fn hit(&mut self, time: SystemTime) {
        self.hit = Some(time);
        //println!("tom {:?} hit! at {:?}", self.id, time);
        //self.hit = Some(time);
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
        //println!("tom {:?} velocity changed to {:?} at {:?}!", self.id, velocity, time);

        // Should we output hit event?
        if let Some(time_of_hit) = self.hit {
            println!("{:?}", time.duration_since(time_of_hit).unwrap().as_micros());
            self.hit = None;
            // 0x90 = note on on channel 1; 0x91 is note on on channel 2
            Some(Message::new(time, 0x90, self.id, velocity))
        } else {
            None
        }
    }
}

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

struct Processor {
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

unsafe impl Send for Processor {}
unsafe impl Sync for Processor {}

impl jack::ProcessHandler for Processor {
    fn process(&mut self, _: &jack::Client, process_scope: &jack::ProcessScope) -> jack::Control {

        //println!("{:?}", process_scope.cycle_times());

        while let Some(Event { id: _, event, time }) = self.gilrs.lock().unwrap().next_event() {
            if let Some(message) = self.drums.process_event(event, time) {
                let event_usecs = SystemTime::now().duration_since(message.time).unwrap().as_micros();
                let period_usecs = process_scope.cycle_times().unwrap().period_usecs;
                if event_usecs >= period_usecs as u128 {
                    println!("{:?} {:?}", event_usecs, period_usecs);
                }
                // TODO - send message into channel here
                //println!("{:?}", message);
            }
        }

        jack::Control::Continue
    }
}

fn main() {

    let (client, _status) = jack::Client::new("Forex", jack::ClientOptions::NO_START_SERVER).unwrap();

    let processor = Processor::new(&client);

    let _active_client = client.activate_async((), processor);


    // Wait for user to input string
    let mut user_input = String::new();
    io::stdin().read_line(&mut user_input).ok();
}

