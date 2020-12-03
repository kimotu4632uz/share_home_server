#![feature(proc_macro_hygiene, decl_macro)]
use rocket::{post, routes, Handler, Request, Response, Route, Data};
use rocket::http::{ContentType, Status, Method};
use rocket::handler::Outcome;
use rocket::response::status::Custom;

use rocket_contrib::serve::{StaticFiles, Options};
use multipart::server::{Multipart, save::{SaveResult, PartialReason}};
use chrono::{DateTime, Local};
use itertools::Itertools;

use std::path::PathBuf;
use std::io::Cursor;
use std::fmt;
use std::fs::DirEntry;

fn resolve_post(ct: &ContentType, path: PathBuf, data: Data) -> Result<String, Custom<String>> {
    let (_, boundary) = ct.params().find(|&(k,_)| k == "boundary").ok_or(
        Custom(Status::BadRequest, "Content-Type: form-data without boundary".into())
    )?;

    let mut entry = Multipart::with_body(data.open(), boundary).into_entry().into_result().map_err(|err| {
        Custom(Status::InternalServerError, err.to_string())
    })?.ok_or(
        Custom(Status::BadRequest, "Request body don't include any entry".into())
    )?;

    let name = entry.headers.filename.ok_or(
        Custom(Status::BadRequest, "Request body don't include filename".into())
    )?;

    let path = dirs::home_dir().unwrap().join(path).join(name);

    match entry.data.save().with_path(path) {
        SaveResult::Full(_) => Ok("File saved".into()),
        SaveResult::Partial(_, err) => match err {
            PartialReason::CountLimit => Err(Custom(Status::InternalServerError, "The count limit for files in the request was hit.".into())),
            PartialReason::SizeLimit => Err(Custom(Status::InternalServerError, "The size limit for an individual file was hit.".into())),
            PartialReason::IoError(e) => Err(Custom(Status::InternalServerError, e.to_string())),
            PartialReason::Utf8Error(_) => Err(Custom(Status::InternalServerError, "UTF convert err".into()))
        },
        SaveResult::Error(err) => Err(Custom(Status::InternalServerError, err.to_string())),
    }
}

#[post("/", format = "multipart/form-data", data = "<data>")]
fn post_root(ct: &ContentType, data: Data) -> Result<String, Custom<String>> {
    resolve_post(ct, PathBuf::default(), data)
}

#[post("/<path..>", format = "multipart/form-data", data = "<data>")]
fn post_other(ct: &ContentType, path: PathBuf, data: Data) -> Result<String, Custom<String>> {
    resolve_post(ct, path, data)
}


enum EntryType {
    File,
    Directory,
}

impl fmt::Display for EntryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File => write!(f, "file"),
            Self::Directory => write!(f, "directory")
        }
    }
}

struct EntryDetail {
    name: String,
    path: PathBuf,
    entry_type: EntryType,
    size: Option<u64>,
    date: Option<DateTime<Local>>,
}

impl EntryDetail {
    fn new(name: String, path: PathBuf, entry_type: EntryType, size: Option<u64>, date: Option<DateTime<Local>>) -> Self {
        EntryDetail { name, path, entry_type, size, date }
    }

    fn to_html(self) -> String {
        format!("<li><a href=\"{}\" class=\"icon icon-{}\" title=\"{}\"><span class=\"name\">{}</span><span class=\"size\">{}</span><span class=\"date\">{}</span></a></li>",
            PathBuf::from("/").join(pathdiff::diff_paths(self.path, dirs::home_dir().unwrap()).unwrap()).to_str().unwrap_or_default(),
            self.entry_type,
            self.name,
            self.name,
            if let Some(s) = self.size { format!("{}", s) } else { "".into() },
            if let Some(d) = self.date { format!("{}", d) } else { "".into() }
        )
    }
}

impl From<DirEntry> for EntryDetail {
    fn from(from: DirEntry) -> Self {
        let name = from.file_name().into_string().unwrap();
        let path = from.path();
        let entry_type = from.file_type().unwrap();
        let entry_type = if entry_type.is_dir() || entry_type.is_symlink() { EntryType::Directory } else { EntryType::File };

        let size = if let EntryType::File = entry_type {
            from.metadata().map(|x| x.len()).ok()
        } else { None };

        let date = if let EntryType::File = entry_type {
            from.metadata().and_then(|x| x.modified()).ok().map(|x| x.into())
        } else { None };

        Self { name, path, entry_type, size, date }
    }
}


#[derive(Clone)]
struct ServeIndex();

impl Handler for ServeIndex {
    fn handle<'r>(&self, req: &'r Request, data: Data) -> Outcome<'r> {
        let path = &req.uri().to_normalized().path()[1..].to_string();
        let path = if path.len() > 0 { PathBuf::from(path) } else { PathBuf::default() };

        let target = dirs::home_dir().unwrap().join(path);

        if !target.is_dir() {
            Outcome::Forward(data)
        } else {
            let mut html = String::new();
            if target != dirs::home_dir().unwrap() {
                html += &EntryDetail::new("..".into(), target.parent().unwrap().into(), EntryType::Directory, None, None).to_html()
            }

            html += &target.read_dir().unwrap().filter_map(Result::ok).filter(|x| x.file_type().unwrap().is_dir()).sorted_by(|x, y| Ord::cmp(&x.file_name(), &y.file_name())).map(|x| EntryDetail::from(x).to_html()).join("");
            html += &target.read_dir().unwrap().filter_map(Result::ok).filter(|x| x.file_type().unwrap().is_file()).sorted_by(|x, y| Ord::cmp(&x.file_name(), &y.file_name())).map(|x| EntryDetail::from(x).to_html()).join("");

            let result = share_home_server::make_html::make_html(html, target);
            let resp = Response::build().sized_body(Cursor::new(result)).finalize();
            Outcome::Success(resp)
        }
    }
}

impl Into<Vec<Route>> for ServeIndex {
    fn into(self) -> Vec<Route> {
        vec![Route::ranked(1, Method::Get, "/", self.clone()), Route::ranked(1, Method::Get, "/<path..>", self)]
    }
}

fn main() {
    rocket::ignite()
        .mount("/", ServeIndex())
        .mount("/", 
            StaticFiles::new(dirs::home_dir().expect("Error: cannot get HOME dir"), Options::DotFiles).rank(10)
        )
        .mount("/", routes![post_root, post_other])
        .launch();
}
