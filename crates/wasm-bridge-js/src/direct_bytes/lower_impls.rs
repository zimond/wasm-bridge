use wasm_bindgen::JsValue;

use super::*;

impl Lower for i32 {
    fn to_abi<M: WriteableMemory>(&self, _memory: M, args: &mut Vec<JsValue>) {
        args.push((*self).into());
    }

    fn write_to<M: WriteableMemory>(&self, _memory: M, memory_slice: &mut M::Slice) {
        memory_slice.write(&self.to_le_bytes())
    }
}

impl Lower for u32 {
    fn to_abi<M: WriteableMemory>(&self, _memory: M, args: &mut Vec<JsValue>) {
        args.push((*self).into());
    }

    fn write_to<M: WriteableMemory>(&self, _memory: M, memory_slice: &mut M::Slice) {
        memory_slice.write(&self.to_le_bytes())
    }
}

impl<T: Lower> Lower for Vec<T> {
    fn to_abi<M: WriteableMemory>(&self, memory: M, args: &mut Vec<JsValue>) {
        let addr = write_vec_data(self, memory) as u32;
        let len = self.len() as u32;

        // First address, then element count
        args.push(addr.into());
        args.push(len.into());
    }

    fn write_to<M: WriteableMemory>(&self, mut memory: M, memory_slice: &mut M::Slice) {
        let addr = write_vec_data(self, &mut memory) as u32;
        let len = self.len() as u32;

        addr.write_to(&mut memory, memory_slice);
        len.write_to(memory, memory_slice);
    }
}

fn write_vec_data<T: Lower, M: WriteableMemory>(data: &[T], mut memory: M) -> usize {
    // Allocate space for all the elements
    let mut slice = memory.allocate(T::alignment(), T::flat_byte_size() * data.len());

    // Then write the elements to the slice buffer
    for elem in data {
        elem.write_to(&mut memory, &mut slice);
    }

    // Then actually write the slice buffer to memory
    memory.flush(slice)
}
