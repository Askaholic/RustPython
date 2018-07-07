
// A function which takes CPython bytecode (from json) and transforms
// this into RustPython bytecode. This to decouple RustPython from CPython
// internal bytecode representations.

use super::bytecode::{self, CodeObject, Instruction};

pub fn convert(cpython_bytecode: CPythonByteCode) -> ByteCode {
    let c = Converter::new();
    c.convert(c)
}


// TODO: think of an appropriate name for this thing:
pub struct Converter {
    frames: Vec<Frame>,
}

impl Converter {
    pub fn new() -> Converter {
        Converter {
            frames: vec![],
        }
    }

    fn curr_frame(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap()
    }

    fn pop_frame(&mut self) {
        self.frames.pop().unwrap();
    }

    fn unwind(&mut self, reason: String) {
        let curr_frame = self.curr_frame();
        let curr_block = curr_frame.blocks[curr_frame.blocks.len()-1].clone(); // use last?
        curr_frame.why = reason; // Why do we need this?
        debug!("block status: {:?}, {:?}", curr_block.block_type, curr_frame.why);
        match (curr_block.block_type.as_ref(), curr_frame.why.as_ref()) {
            ("loop", "break") => {
                curr_frame.lasti = curr_block.handler; //curr_frame.labels[curr_block.handler]; // Jump to the end
                // Return the why as None
                curr_frame.blocks.pop();
            },
            ("loop", "none") => (), //skipped
            _ => panic!("block stack operation not implemented")
        }
    }

    // Can we get rid of the code paramter?

    fn make_frame(&self, code: PyCodeObject, callargs: HashMap<String, Rc<NativeType>>, globals: Option<HashMap<String, Rc<NativeType>>>) -> Frame {
        //populate the globals and locals
        let mut labels = HashMap::new();
        let mut curr_offset = 0;
        for (idx, op) in code.co_code.iter().enumerate() {
            labels.insert(curr_offset, idx);
            curr_offset += op.0;
        }
        //TODO: This is wrong, check https://github.com/nedbat/byterun/blob/31e6c4a8212c35b5157919abff43a7daa0f377c6/byterun/pyvm2.py#L95
        let globals = match globals {
            Some(g) => g,
            None => HashMap::new(),
        };
        let mut locals = globals;
        locals.extend(callargs);

        //TODO: move this into the __builtin__ module when we have a module type
        locals.insert("print".to_string(), Rc::new(NativeType::NativeFunction(builtins::print)));
        locals.insert("len".to_string(), Rc::new(NativeType::NativeFunction(builtins::len)));
        Frame {
            code: code,
            stack: vec![],
            blocks: vec![],
            // save the callargs as locals
            globals: locals.clone(),
            locals: locals,
            labels: labels,
            lasti: 0,
            return_value: NativeType::NoneType,
            why: "none".to_string(),
        }
    }

    // The Option<i32> is the return value of the frame, remove when we have implemented frame
    // TODO: read the op codes directly from the internal code object
    fn run_frame(&mut self, frame: Frame) -> NativeType {
        self.frames.push(frame);

        //let mut why = None;
        // Change this to a loop for jump
        loop {
            //while curr_frame.lasti < curr_frame.code.co_code.len() {
            let op_code = {
                let curr_frame = self.curr_frame();
                if curr_frame.code.co_code.len() == 0 { panic!("Trying to run an empty frame. Check if the bytecode is empty"); }
                let op_code = curr_frame.code.co_code[curr_frame.lasti].clone();
                curr_frame.lasti += 1;
                op_code
            };
            let why = self.dispatch(op_code);
            /*if curr_frame.blocks.len() > 0 {
              self.manage_block_stack(&why);
              }
              */
            if let Some(_) = why {
                break;
            }
        }
        let return_value = {
            //let curr_frame = self.frames.last_mut().unwrap();
            self.curr_frame().return_value.clone()
        };
        self.pop_frame();
        return_value
    }

    pub fn run_code(&mut self, code: PyCodeObject) {
        let frame = self.make_frame(code, HashMap::new(), None);
        self.run_frame(frame);
        // check if there are any leftover frame, fail if any
    }

    fn dispatch(&mut self, op_code: (usize, String, Option<usize>)) {
        {
            debug!("stack:{:?}", self.curr_frame().stack);
            debug!("env  :{:?}", self.curr_frame().locals);
            debug!("Executing op code: {:?}", op_code);
        }
        match (op_code.1.as_ref(), op_code.2) {
            ("LOAD_CONST", Some(consti)) => {
                // println!("Loading const at index: {}", consti);
                let curr_frame = self.curr_frame();
                let value = curr_frame.code.co_consts[consti].clone();
                self.emit(LoadConst(None));
            },

            // TODO: universal stack element type
            ("LOAD_CONST", None) => {
                self.emit(LoadConst(None));
            },
            ("POP_TOP", None) => {
                self.emit(Pop(None));
            },
            ("LOAD_FAST", Some(var_num)) => {
                let curr_frame = self.curr_frame();
                let ref name = curr_frame.code.co_varnames[var_num];
                curr_frame.stack.push(curr_frame.locals.get::<str>(name).unwrap().clone());
            },
            ("STORE_NAME", Some(namei)) => {
                // println!("Loading const at index: {}", consti);
                let curr_frame = self.curr_frame();
                curr_frame.locals.insert(curr_frame.code.co_names[namei].clone(), curr_frame.stack.pop().unwrap().clone());
            },
            ("LOAD_NAME", Some(namei)) => {
                // println!("Loading const at index: {}", consti);
                let curr_frame = self.curr_frame();
                &curr_frame.code.co_names[namei]
                let name = code.clone();
                self.emit(Instruction::LoadName { name });
            },
            ("LOAD_GLOBAL", Some(namei)) => {
                // We need to load the underlying value the name points to, but stuff like
                // AssertionError is in the names right after compile, so we load the string
                // instead for now
                let curr_frame = self.curr_frame();
                let name = &curr_frame.code.co_names[namei];
                curr_frame.stack.push(curr_frame.globals.get::<str>(name).unwrap().clone());
            },

            ("BUILD_LIST", Some(count)) => {
                self.emit(Instruction::BuildList { size: count });
            },

            ("BUILD_SLICE", Some(count)) => {
                let curr_frame = self.curr_frame();
                assert!(count == 2 || count == 3);
                let mut vec = vec!();
                for _ in 0..count {
                    vec.push(curr_frame.stack.pop().unwrap());
                }
                vec.reverse();
                let mut out:Vec<Option<i32>> = vec.into_iter().map(|x| match *x {
                    NativeType::Int(n) => Some(n),
                    NativeType::NoneType => None,
                    _ => panic!("Expect Int or None as BUILD_SLICE arguments, got {:?}", x),
                }).collect();

                if out.len() == 2 {
                    out.push(None);
                }
                assert!(out.len() == 3);
                // TODO: assert the stop start and step are NativeType::Int
                // See https://users.rust-lang.org/t/how-do-you-assert-enums/1187/8
                curr_frame.stack.push(Rc::new(NativeType::Slice(out[0], out[1], out[2])));
                None
            },

            ("GET_ITER", None) => {
                self.emit(Instruction::GetIter);
            },

            ("FOR_ITER", Some(delta)) => {
                self.emit(Instruction::ForIter);
            },

            ("COMPARE_OP", Some(cmp_op_i)) => {
                let curr_frame = self.curr_frame();
                let v1 = curr_frame.stack.pop().unwrap();
                let v2 = curr_frame.stack.pop().unwrap();
                match CMP_OP[cmp_op_i] {
                    // To avoid branch explotion, use an array of callables instead
                    "==" => {
                        match (v1.deref(), v2.deref()) {
                            (&NativeType::Int(ref v1i), &NativeType::Int(ref v2i)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2i == v1i)));
                            },
                            (&NativeType::Float(ref v1f), &NativeType::Float(ref v2f)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2f == v1f)));
                            },
                            (&NativeType::Str(ref v1s), &NativeType::Str(ref v2s)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2s == v1s)));
                            },
                            (&NativeType::Int(ref v1i), &NativeType::Float(ref v2f)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2f == &(*v1i as f64))));
                            },
                            (&NativeType::List(ref l1), &NativeType::List(ref l2)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(l2 == l1)));
                            },
                            _ => panic!("TypeError in COMPARE_OP: can't compare {:?} with {:?}", v1, v2)
                        };
                    }
                    ">" => {
                        match (v1.deref(), v2.deref()) {
                            (&NativeType::Int(ref v1i), &NativeType::Int(ref v2i)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2i < v1i)));
                            },
                            (&NativeType::Float(ref v1f), &NativeType::Float(ref v2f)) => {
                                curr_frame.stack.push(Rc::new(NativeType::Boolean(v2f < v1f)));
                            },
                            _ => panic!("TypeError in COMPARE_OP")
                        };
                    }
                    _ => panic!("Unimplemented COMPARE_OP operator")

                }
                None
                
            },
            ("POP_JUMP_IF_TRUE", Some(ref target)) => {
                let curr_frame = self.curr_frame();
                let v = curr_frame.stack.pop().unwrap();
                if *v == NativeType::Boolean(true) {
                    curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                }
                None

            }
            ("POP_JUMP_IF_FALSE", Some(ref target)) => {
                let curr_frame = self.curr_frame();
                let v = curr_frame.stack.pop().unwrap();
                if *v == NativeType::Boolean(false) {
                    curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                }
                None
                
            }
            ("JUMP_FORWARD", Some(ref delta)) => {
                let curr_frame = self.curr_frame();
                let last_offset = curr_frame.get_bytecode_offset().unwrap();
                curr_frame.lasti = curr_frame.labels.get(&(last_offset + delta)).unwrap().clone();
                None
            },
            ("JUMP_ABSOLUTE", Some(ref target)) => {
                let curr_frame = self.curr_frame();
                curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                None
            },
            ("BREAK_LOOP", None) => {
                // Do we still need to return the why if we use unwind from jsapy?
                self.unwind("break".to_string());
                None //?
            },
            ("RAISE_VARARGS", Some(argc)) => {
                let curr_frame = self.curr_frame();
                // let (exception, params, traceback) = match argc {
                let exception = match argc {
                    1 => curr_frame.stack.pop().unwrap(),
                    0 | 2 | 3 => panic!("Not implemented!"),
                    _ => panic!("Invalid paramter for RAISE_VARARGS, must be between 0 to 3")
                };
                panic!("{:?}", exception);
            }
            ("INPLACE_ADD", None) => {
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let tos1 = curr_frame.stack.pop().unwrap();
                match (tos.deref(), tos1.deref()) {
                    (&NativeType::Int(ref tosi), &NativeType::Int(ref tos1i)) => {
                        curr_frame.stack.push(Rc::new(NativeType::Int(tos1i + tosi)));
                    },
                    _ => panic!("TypeError in BINARY_ADD")
                }
                None
            },
            
            ("STORE_SUBSCR", None) => {
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let tos1 = curr_frame.stack.pop().unwrap();
                let tos2 = curr_frame.stack.pop().unwrap();
                match (tos1.deref(), tos.deref()) {
                    (&NativeType::List(ref refl), &NativeType::Int(index)) => {
                        refl.borrow_mut()[index as usize] = (*tos2).clone();
                    },
                    (&NativeType::Str(_), &NativeType::Int(_)) => {
                        // TODO: raise TypeError: 'str' object does not support item assignment
                        panic!("TypeError: 'str' object does not support item assignment")
                    },
                    _ => panic!("TypeError in STORE_SUBSCR")
                }
                curr_frame.stack.push(tos1);
                None
            },

            ("BINARY_ADD", None) => {
                self.emit(Instruction::BinaryOperation { op: BinaryOperator::Add });
            },
            ("BINARY_POWER", None) => {
                self.emit(Instruction::BinaryOperation { op: BinaryOperator::Power });
            },
            ("BINARY_MULTIPLY", None) => {
                self.emit(Instruction::BinaryOperation { op: BinaryOperator::Multiply });
            },
            ("BINARY_TRUE_DIVIDE", None) => {
                self.emit(Instruction::BinaryOperation { op: BinaryOperator::TrueDivide });
            },
            ("BINARY_MODULO", None) => {
                self.emit(Instruction::BinaryOperation { op: BinaryOperator::Modulo });
            },
            ("BINARY_SUBTRACT", None) => {
                self.emit(Instruction::BinaryOperation { op: BinaryOperator::Subtract });
            },

            ("ROT_TWO", None) => {
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let tos1 = curr_frame.stack.pop().unwrap();
                curr_frame.stack.push(tos);
                curr_frame.stack.push(tos1);
                None
            }
            ("UNARY_NEGATIVE", None) => {
                self.emit(Instruction::UnaryOperation { op: UnaryOperator::Minus });
            },
            ("UNARY_POSITIVE", None) => {
                self.emit(Instruction::UnaryOperation { op: UnaryOperator::Plus });
            },
            ("PRINT_ITEM", None) => {
                let curr_frame = self.curr_frame();
                // TODO: Print without the (...)
                println!("{:?}", curr_frame.stack.pop().unwrap());
            },
            ("PRINT_NEWLINE", None) => {
                print!("\n");
            },
            ("MAKE_FUNCTION", Some(argc)) => {
                // https://docs.python.org/3.4/library/dis.html#opcode-MAKE_FUNCTION
                let curr_frame = self.curr_frame();
                let qualified_name = curr_frame.stack.pop().unwrap();
                let code_obj = match curr_frame.stack.pop().unwrap().deref() {
                    &NativeType::Code(ref code) => code.clone(),
                    _ => panic!("Second item on the stack should be a code object")
                };
                // pop argc arguments
                // argument: name, args, globals
                let func = Function::new(code_obj);
                curr_frame.stack.push(Rc::new(NativeType::Function(func)));
                None
            },
            ("CALL_FUNCTION", Some(argc)) => {
                let kw_count = (argc >> 8) as u8;
                let pos_count = (argc & 0xFF) as u8;
                // Pop the arguments based on argc
                let mut kw_args = HashMap::new();
                let mut pos_args = Vec::new();
                {
                    let curr_frame = self.curr_frame();
                    for _ in 0..kw_count {
                        let native_val = curr_frame.stack.pop().unwrap();
                        let native_key = curr_frame.stack.pop().unwrap();
                        if let (ref val, &NativeType::Str(ref key)) = (native_val, native_key.deref()) {

                            kw_args.insert(key.clone(), val.clone());
                        }
                        else {
                            panic!("Incorrect type found while building keyword argument list")
                        }
                    }
                    for _ in 0..pos_count {
                        pos_args.push(curr_frame.stack.pop().unwrap());
                    }
                }
                let locals = {
                    // FIXME: no clone here
                    self.curr_frame().locals.clone()
                };

                let func = {
                    match self.curr_frame().stack.pop().unwrap().deref() {
                        &NativeType::Function(ref func) => {
                            // pop argc arguments
                            // argument: name, args, globals
                            // build the callargs hashmap
                            pos_args.reverse();
                            let mut callargs = HashMap::new();
                            for (name, val) in func.code.co_varnames.iter().zip(pos_args) {
                                callargs.insert(name.to_string(), val);
                            }
                            // merge callargs with kw_args
                            let return_value = {
                                let frame = self.make_frame(func.code.clone(), callargs, Some(locals));
                                self.run_frame(frame)
                            };
                            self.curr_frame().stack.push(Rc::new(return_value));
                        },
                        &NativeType::NativeFunction(func) => {
                            pos_args.reverse();
                            let return_value = func(pos_args);
                            self.curr_frame().stack.push(Rc::new(return_value));
                        },
                        _ => panic!("The item on the stack should be a code object")
                    }
                };
                None
            },
            ("RETURN_VALUE", None) => {
                // Hmmm... what is this used?
                // I believe we need to push this to the next frame
                self.curr_frame().return_value = (*self.curr_frame().stack.pop().unwrap()).clone();
                Some("return".to_string())
            },
            ("SETUP_LOOP", Some(delta)) => {
                let curr_frame = self.curr_frame();
                let curr_offset = curr_frame.get_bytecode_offset().unwrap();
                curr_frame.blocks.push(Block {
                    block_type: "loop".to_string(),
                    handler: *curr_frame.labels.get(&(curr_offset + delta)).unwrap(),
                });
                None
            },
            ("POP_BLOCK", None) => {
                self.curr_frame().blocks.pop();
                None
            }
            ("SetLineno", _) | ("LABEL", _)=> {
                // Skip
                None
            },
            (name, _) => {
                panic!("Unrecongnizable op code: {}", name);
            }
        } // end match
    } // end dispatch function

    fn emit(&mut self, instruction: Instruction) {
        self.code_object.instructions.push(instruction);
    }
}
