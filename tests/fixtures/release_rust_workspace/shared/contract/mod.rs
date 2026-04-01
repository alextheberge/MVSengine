pub struct Hidden;

impl Hidden {
    pub fn ping(&self) -> bool {
        true
    }
}

pub fn connect(target: u32) -> bool {
    target > 0
}
