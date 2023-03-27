use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

enum NodeKind {
    File(Arc<Vec<u8>>),
    Directory(BTreeMap<String, Arc<Node>>),
}

struct Node {
    parent: Weak<Node>,
    name: String,
    kind: Mutex<NodeKind>,
}

enum VfsErr {
    NotDir { path: String },
    IsDirectory { path: String },
}

impl std::fmt::Display for VfsErr {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match *self {
            VfsErr::NotDir { ref path } => {
                write!(fmt, "is not a directory: \"{}\"", path)
            }
            VfsErr::IsDirectory { ref path } => {
                write!(fmt, "is a directory: \"{}\"", path)
            }
        }
    }
}

impl From<VfsErr> for Box<rhai::EvalAltResult> {
    fn from(err: VfsErr) -> Self {
        Box::new(err.to_string().into())
    }
}

impl Node {
    pub fn parent(&self) -> Option<Arc<Node>> {
        self.parent.upgrade()
    }

    pub fn absolute_path_into(&self, output: &mut String) {
        if let Some(parent) = self.parent() {
            parent.absolute_path_into(output);
            if !output.ends_with('/') {
                output.push('/');
            }
            output.push_str(&self.name);
        } else {
            output.push('/');
        }
    }

    pub fn absolute_path(&self) -> String {
        let mut output = String::new();
        self.absolute_path_into(&mut output);
        output
    }

    pub fn child_path(&self, name: &str) -> String {
        let mut output = String::new();
        self.absolute_path_into(&mut output);
        if !output.ends_with('/') {
            output.push('/');
        }
        output.push_str(name);
        output
    }

    pub fn get_child(&self, name: &str) -> Result<Option<Arc<Node>>, VfsErr> {
        match *self.kind.lock() {
            NodeKind::File(..) => Err(VfsErr::NotDir {
                path: self.absolute_path(),
            }),
            NodeKind::Directory(ref directory) => Ok(directory.get(name).cloned()),
        }
    }

    pub fn get_directory_by_relative_path(
        self: &Arc<Node>,
        path: &str,
    ) -> Result<Arc<Node>, VfsErr> {
        if path.is_empty() {
            return Ok(self.clone());
        }

        let mut node = self.clone();
        for chunk in path.split("/") {
            node = match node.get_child(chunk)? {
                Some(node) => node,
                None => {
                    return Err(VfsErr::NotDir {
                        path: node.child_path(chunk),
                    });
                }
            }
        }

        if !node.is_dir() {
            return Err(VfsErr::NotDir { path: path.into() }.into());
        }

        Ok(node)
    }

    pub fn is_dir(&self) -> bool {
        match *self.kind.lock() {
            NodeKind::File(..) => false,
            NodeKind::Directory(..) => true,
        }
    }

    pub fn mkdir(self: &Arc<Node>, name: &str) -> Result<Arc<Node>, VfsErr> {
        match *self.kind.lock() {
            NodeKind::File(..) => Err(VfsErr::NotDir {
                path: self.absolute_path(),
            }),
            NodeKind::Directory(ref mut directory) => {
                if let Some(child) = directory.get(name) {
                    if !child.is_dir() {
                        return Err(VfsErr::NotDir {
                            path: self.child_path(name),
                        });
                    }
                    return Ok(child.clone());
                }

                let node = Arc::new(Node {
                    parent: Arc::downgrade(&self),
                    name: name.to_owned(),
                    kind: Mutex::new(NodeKind::Directory(Default::default())),
                });

                directory.insert(name.to_owned(), node.clone());
                Ok(node)
            }
        }
    }

    pub fn get_or_create_file(self: &Arc<Node>, filename: &str) -> Result<Arc<Node>, VfsErr> {
        match *self.kind.lock() {
            NodeKind::File(..) => Err(VfsErr::NotDir {
                path: self.absolute_path(),
            }),
            NodeKind::Directory(ref mut directory) => {
                if let Some(child) = directory.get(filename) {
                    match *child.kind.lock() {
                        NodeKind::Directory(..) => Err(VfsErr::IsDirectory {
                            path: self.child_path(filename),
                        }),
                        NodeKind::File(..) => Ok(child.clone()),
                    }
                } else {
                    let node = Arc::new(Node {
                        parent: Arc::downgrade(&self),
                        name: filename.to_owned(),
                        kind: Mutex::new(NodeKind::File(Default::default())),
                    });

                    directory.insert(filename.to_owned(), node.clone());
                    Ok(node)
                }
            }
        }
    }
}

pub enum ScriptOutputKind {
    PrintLine(String),
    Image { path: String, data: Arc<Vec<u8>> },
}

pub struct VirtualEnvironment {
    cwd: String,
    root: Arc<Node>,
    pub output: Vec<ScriptOutputKind>,
}

impl VirtualEnvironment {
    pub fn new() -> Self {
        VirtualEnvironment {
            cwd: "/".into(),
            root: Arc::new(Node {
                parent: Weak::new(),
                name: "".into(),
                kind: Mutex::new(NodeKind::Directory(Default::default())),
            }),
            output: Default::default(),
        }
    }

    fn normalize_path(&self, mut path: &str) -> String {
        if path == "/" {
            return path.into();
        }

        if path.ends_with("/") {
            path = &path[..path.len() - 1];
        }

        if path.starts_with("/") {
            return path.into();
        }

        let mut output = String::with_capacity(self.cwd.len() + path.len() + 1);
        output.push_str(&self.cwd);
        if !output.ends_with("/") {
            output.push('/');
        }
        output.push_str(path);
        output
    }
}

impl crate::script::Environment for VirtualEnvironment {
    fn println(&mut self, message: &str) {
        self.output
            .push(ScriptOutputKind::PrintLine(message.into()));
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), Box<rhai::EvalAltResult>> {
        let path = self.normalize_path(path);
        let mut node = self.root.clone();
        for chunk in path[1..].split("/") {
            node = node.mkdir(chunk)?;
        }
        Ok(())
    }

    fn chdir(&mut self, path: &str) -> Result<(), Box<rhai::EvalAltResult>> {
        let path = self.normalize_path(path);
        self.root.get_directory_by_relative_path(&path[1..])?;
        self.cwd = path.into();
        Ok(())
    }

    fn file_write(
        &mut self,
        path: &str,
        kind: crate::script::FileKind,
        contents: &[u8],
    ) -> Result<(), Box<rhai::EvalAltResult>> {
        let path = self.normalize_path(path);
        let index = path.rfind("/").unwrap();
        let dirname = &path[..std::cmp::max(1, index)];
        let filename = &path[index + 1..];
        if filename.is_empty() {
            return Err(crate::script::error("missing filename"));
        }

        let directory = self.root.get_directory_by_relative_path(&dirname[1..])?;
        let child = if let Some(node) = directory.get_child(filename)? {
            if node.is_dir() {
                return Err(VfsErr::IsDirectory { path }.into());
            }

            node
        } else {
            directory.get_or_create_file(filename)?
        };

        let contents: Vec<u8> = contents.into();
        let contents = Arc::new(contents);
        *child.kind.lock() = NodeKind::File(contents.clone());

        use crate::script::FileKind;
        match kind {
            FileKind::Svg => {
                self.output.push(ScriptOutputKind::Image {
                    path,
                    data: contents,
                });
            }
        }

        Ok(())
    }
}
