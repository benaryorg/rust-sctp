use std;
use libc;
use sctp_sys;

use std::io::{Result, Error, ErrorKind, Read, Write};
use std::net::{ToSocketAddrs, SocketAddr, Shutdown};
use std::mem::{transmute, size_of, zeroed};

#[cfg(target_os="linux")]
use std::os::unix::io::{AsRawFd, RawFd, FromRawFd};
#[cfg(target_os="windows")]
use std::os::windows::io::{AsRawHandle, RawHandle, FromRawHandle};

#[cfg(target_os="windows")]
pub type SOCKET = libc::SOCKET;
#[cfg(target_os="linux")]
pub type SOCKET = libc::c_int;

#[cfg(target_os="windows")]
type RWlen = i32;
#[cfg(target_os="linux")]
type RWlen = libc::size_t;

#[cfg(target_os="windows")]
use libc::closesocket;
#[cfg(target_os="linux")]
unsafe fn closesocket(sock: SOCKET) {
	libc::close(sock);
}

#[cfg(target_os="windows")]
fn check_socket(sock: SOCKET) -> Result<SOCKET> {
	if sock == libc::INVALID_SOCKET { return Err(Error::last_os_error()); }
	return Ok(sock);
}

#[cfg(target_os="linux")]
fn check_socket(sock: SOCKET) -> Result<SOCKET> {
	if sock < 0 { return Err(Error::last_os_error()); }
	return Ok(sock);
}

// XXX: Until getsockopt is available in libc crate
extern "system" {
	#[cfg(target_os="linux")]
	fn getsockopt(sock: SOCKET, level: libc::c_int, optname: libc::c_int, optval: *mut libc::c_void, optlen: *mut libc::socklen_t) -> libc::c_int;
	#[cfg(target_os="windows")]
	fn getsockopt(sock: SOCKET, level: libc::c_int, optname: libc::c_int, optval: *mut libc::c_char, optlen: *mut libc::c_int) -> libc::c_int;
}

/// SCTP bind operation
#[allow(dead_code)]
pub enum BindOp {
	/// Add bind addresses
	AddAddr,
	/// Remove bind addresses
	RemAddr
}

impl BindOp {
	fn flag(&self) -> libc::c_int {
		return match *self {
			BindOp::AddAddr => sctp_sys::SCTP_BINDX_ADD_ADDR,
			BindOp::RemAddr => sctp_sys::SCTP_BINDX_REM_ADDR
		};
	}
}

enum SctpAddrType {
	Local,
	Peer
}

impl SctpAddrType {
	unsafe fn get(&self, sock: SOCKET, id: sctp_sys::sctp_assoc_t, ptr: *mut *mut libc::sockaddr) -> libc::c_int {
		return match *self {
			SctpAddrType::Local => sctp_sys::sctp_getladdrs(sock, id, ptr),
			SctpAddrType::Peer => sctp_sys::sctp_getpaddrs(sock, id, ptr)
		};
	}
	
	unsafe fn free(&self, ptr: *mut libc::sockaddr) {
		return match *self {
			SctpAddrType::Local => sctp_sys::sctp_freeladdrs(ptr),
			SctpAddrType::Peer => sctp_sys::sctp_freepaddrs(ptr)
		};
	}
}


/// Manage low level socket address structure
pub trait RawSocketAddr {
	/// Get the address family for this socket address
	fn family(&self) -> i32;
	
	/// Get the raw socket address structure size
	fn addr_len(&self) -> libc::socklen_t;
	
	/// Create from a raw socket address 
	unsafe fn from_raw_ptr(addr: *const libc::sockaddr, len: libc::socklen_t) -> Result<Self>;
	
	/// Return an immutable pointer to the raw socket address structure
	fn as_ptr(&self) -> *const libc::sockaddr;
	
	/// Return a mutable pointer to the raw socket address structure
	fn as_mut_ptr(&mut self) -> *mut libc::sockaddr;
	
	/// Create from a ToSocketAddrs
	fn from_addr<A: ToSocketAddrs>(address: A) -> Result<Self>;
}

impl RawSocketAddr for SocketAddr {
	fn family(&self) -> i32 {
		return match *self {
			SocketAddr::V4(..) => libc::AF_INET,
			SocketAddr::V6(..) => libc::AF_INET6
		};
	}
	
	fn addr_len(&self) -> libc::socklen_t {
		return match *self {
			SocketAddr::V4(..) => size_of::<libc::sockaddr_in>(),
			SocketAddr::V6(..) => size_of::<libc::sockaddr_in6>()
		} as libc::socklen_t;
	}
	
	unsafe fn from_raw_ptr(addr: *const libc::sockaddr, len: libc::socklen_t) -> Result<SocketAddr> {
		if len < size_of::<libc::sockaddr>() as libc::socklen_t {
			return Err(Error::new(ErrorKind::InvalidInput, "Invalid address length"));
		}
		return match (*addr).sa_family as libc::c_int {
			libc::AF_INET if len >= size_of::<libc::sockaddr_in>() as libc::socklen_t => Ok(SocketAddr::V4(transmute(*(addr as *const libc::sockaddr_in)))),
			libc::AF_INET6 if len >= size_of::<libc::sockaddr_in6>() as libc::socklen_t => Ok(SocketAddr::V6(transmute(*(addr as *const libc::sockaddr_in6)))),
			_ => Err(Error::new(ErrorKind::InvalidInput, "Cannot get peer socket address"))
		};
	}
	
	fn as_ptr(&self) -> *const libc::sockaddr {
		return match *self {
			SocketAddr::V4(ref a) => unsafe { transmute(a) },
			SocketAddr::V6(ref a) => unsafe { transmute(a) }
		};
	}
	
	fn as_mut_ptr(&mut self) -> *mut libc::sockaddr {
		return match *self {
			SocketAddr::V4(ref mut a) => unsafe { transmute(a) },
			SocketAddr::V6(ref mut a) => unsafe { transmute(a) }
		};
	}
	
	fn from_addr<A: ToSocketAddrs>(address: A) -> Result<SocketAddr> {
		return try!(address.to_socket_addrs().or(Err(Error::new(ErrorKind::InvalidInput, "Address is not valid"))))
								.next().ok_or(Error::new(ErrorKind::InvalidInput, "Address is not valid"));
	}
}


/// A High level wrapper around SCTP socket, of any kind
pub struct SctpSocket(SOCKET);

impl SctpSocket {
	/// Create a new SCTP socket
	pub fn new(family: libc::c_int, sock_type: libc::c_int) -> Result<SctpSocket> {
		unsafe {
			return Ok(SctpSocket(try!(check_socket(libc::socket(family, sock_type, sctp_sys::IPPROTO_SCTP)))));
		}
	}
	
	/// Connect the socket to `address`
	pub fn connect<A: ToSocketAddrs>(&self, address: A) -> Result<()> {
		let raw_addr = try!(SocketAddr::from_addr(&address));
		unsafe {
			return match libc::connect(self.0, raw_addr.as_ptr(), raw_addr.addr_len()) {
				0 => Ok(()),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Connect the socket to multiple addresses
	pub fn connectx<A: ToSocketAddrs>(&self, addresses: &[A]) -> Result<sctp_sys::sctp_assoc_t> {
		if addresses.len() == 0 { return Err(Error::new(ErrorKind::InvalidInput, "No addresses given")); }
		unsafe {
			let buf: *mut u8 = libc::malloc((addresses.len() * size_of::<libc::sockaddr_in6>()) as u64) as *mut u8;
			if buf.is_null() {
				return Err(Error::new(ErrorKind::Other, "Out of memory"));
			}
			let mut offset = 0isize;
			for address in addresses {
				let raw = try!(SocketAddr::from_addr(address));
				let len = raw.addr_len();
				std::ptr::copy_nonoverlapping(raw.as_ptr() as *mut u8, buf.offset(offset), len as usize);
				offset += len as isize;
			}
			
			let mut assoc: sctp_sys::sctp_assoc_t = 0;
			let ret = match sctp_sys::sctp_connectx(self.0, buf as *mut libc::sockaddr, addresses.len() as i32, &mut assoc) {
				0 => Ok(assoc),
				_ => Err(Error::last_os_error()),
			};
			libc::free(buf as *mut libc::c_void);
			return ret;
		}
	}
	
	/// Bind the socket to a single address
	pub fn bind<A: ToSocketAddrs>(&self, address: A) -> Result<()> {
		let raw_addr = try!(SocketAddr::from_addr(&address));
		unsafe {
			return match libc::bind(self.0, raw_addr.as_ptr(), raw_addr.addr_len()) {
				0 => Ok(()),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Bind the socket on multiple addresses
	pub fn bindx<A: ToSocketAddrs>(&self, addresses: &[A], op: BindOp) -> Result<()> {
		if addresses.len() == 0 { return Err(Error::new(ErrorKind::InvalidInput, "No addresses given")); }
		unsafe {
			let buf: *mut u8 = libc::malloc((addresses.len() * size_of::<libc::sockaddr_in6>()) as u64) as *mut u8;
			if buf.is_null() {
				return Err(Error::new(ErrorKind::Other, "Out of memory"));
			}
			let mut offset = 0isize;
			for address in addresses {
				let raw = try!(SocketAddr::from_addr(address));
				let len = raw.addr_len();
				std::ptr::copy_nonoverlapping(raw.as_ptr() as *mut u8, buf.offset(offset), len as usize);
				offset += len as isize;
			}

			let ret = match sctp_sys::sctp_bindx(self.0, buf as *mut libc::sockaddr, addresses.len() as i32, op.flag()) {
				0 => Ok(()),
				_ => Err(Error::last_os_error())
			};
			libc::free(buf as *mut libc::c_void);
			return ret;
		}
	}
	
	/// Listen
	pub fn listen(&self, backlog: libc::c_int) -> Result<()> {
		unsafe {
			return match libc::listen(self.0, backlog) {
				0 => Ok(()),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Accept connection to this socket
	pub fn accept(&self) -> Result<(SctpSocket, SocketAddr)> {
		let mut addr: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
		let mut len: libc::socklen_t = size_of::<libc::sockaddr_in6>() as libc::socklen_t;
		unsafe {
			let addr_ptr: *mut libc::sockaddr = transmute(&mut addr);
			let sock = try!(check_socket(libc::accept(self.0, addr_ptr, &mut len)));
			let addr = try!(SocketAddr::from_raw_ptr(addr_ptr, len));
			return Ok((SctpSocket(sock), addr));
		}
	}
	
	fn addrs(&self, id: sctp_sys::sctp_assoc_t, what: SctpAddrType) -> Result<Vec<SocketAddr>> {
		unsafe {
			let mut	addrs: *mut u8 = std::ptr::null_mut();
			let len = what.get(self.0, id, transmute(&mut addrs));
			if len < 0 { return Err(Error::new(ErrorKind::Other, "Cannot retrieve local addresses")); }
			if len == 0 { return Err(Error::new(ErrorKind::AddrNotAvailable, "Socket is unbound")); }
			
			let mut vec = Vec::with_capacity(len as usize);
			let mut offset = 0;
			for _ in 0..len {
				let sockaddr = addrs.offset(offset) as *const libc::sockaddr;
				let len = match (*sockaddr).sa_family as i32 {
					libc::AF_INET => size_of::<libc::sockaddr_in>(),
					libc::AF_INET6 => size_of::<libc::sockaddr_in6>(),
					f => {
						what.free(addrs as *mut libc::sockaddr);
						return Err(Error::new(ErrorKind::Other, format!("Unsupported address family : {}", f)));
					}
				} as libc::socklen_t;
				vec.push(try!(SocketAddr::from_raw_ptr(sockaddr, len)));
				offset += len as isize;
			}
			what.free(addrs as *mut libc::sockaddr);
			return Ok(vec);
		}
	}
	
	/// List socket's local addresses
	pub fn local_addrs(&self, id: sctp_sys::sctp_assoc_t) -> Result<Vec<SocketAddr>> {
		return self.addrs(id, SctpAddrType::Local);
	}
	
	/// Get peer addresses for a connected socket or a given association
	pub fn peer_addrs(&self, id: sctp_sys::sctp_assoc_t) -> Result<Vec<SocketAddr>> {
		return self.addrs(id, SctpAddrType::Peer);
	}
	
	/// Receive data in TCP style. Only works for a connected one to one socket
	pub fn recv(&mut self, buf: &mut [u8]) -> Result<usize> {
		unsafe {
			let len = buf.len() as RWlen;
			return match libc::recv(self.0, buf.as_mut_ptr() as *mut libc::c_void, len, 0) {
				res if res >= 0 => Ok(res as usize),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Send data in TCP style. Only works for a connected one to one socket
	pub fn send(&mut self, buf: &[u8]) -> Result<usize> {
		unsafe {
			let len = buf.len() as RWlen;
			return match libc::send(self.0, buf.as_ptr() as *const libc::c_void, len, 0) {
				res if res >= 0 => Ok(res as usize),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Wait for data to be received. On success, returns a triplet containing
	/// the quantity of bytes received, the sctp stream id on which data were received, and
	/// the socket address used by the peer to send the data
	pub fn recvmsg(&self, msg: &mut [u8]) -> Result<(usize, u16, SocketAddr)> {
		let len = msg.len() as libc::size_t;
		let mut addr: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
		let mut addr_len: libc::socklen_t = size_of::<libc::sockaddr_in6>() as libc::socklen_t;
		let mut flags: libc::c_int = 0;
		unsafe {
			let addr_ptr: *mut libc::sockaddr = transmute(&mut addr);
			let mut info: sctp_sys::sctp_sndrcvinfo = std::mem::zeroed();
			return match sctp_sys::sctp_recvmsg(self.0, msg.as_mut_ptr() as *mut libc::c_void, len, addr_ptr, &mut addr_len, &mut info, &mut flags) {
				res if res > 0 => Ok((res as usize, info.sinfo_stream, try!(SocketAddr::from_raw_ptr(addr_ptr, addr_len)))),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Send data in Sctp style, to the provided address (may be `None` if the socket is connected), on the stream `stream`, with the TTL `ttl`.
	/// On success, returns the quantity on bytes sent
	pub fn sendmsg<A: ToSocketAddrs>(&self, msg: &[u8], address: Option<A>, stream: u16, ttl: libc::c_ulong) -> Result<usize> {
		let len = msg.len() as libc::size_t;
		let (raw_addr, addr_len) = match address {
			Some(a) => {
				let mut addr = try!(SocketAddr::from_addr(a));
				(addr.as_mut_ptr(), addr.addr_len())
			},
			None => (std::ptr::null_mut(), 0)
		};
		unsafe {
			return match sctp_sys::sctp_sendmsg(self.0, msg.as_ptr() as *const libc::c_void, len, raw_addr, addr_len, 0, 0, stream, ttl, 0) {
				res if res > 0 => Ok(res as usize),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Shuts down the read, write, or both halves of this connection
	pub fn shutdown(&self, how: Shutdown) -> Result<()> {
		let side = match how {
			Shutdown::Read => libc::SHUT_RD,
			Shutdown::Write => libc::SHUT_WR,
			Shutdown::Both => libc::SHUT_RDWR
		};
		return match unsafe { libc::shutdown(self.0, side) } {
			0 => Ok(()),
			_ => Err(Error::last_os_error())
		};
	}
	
	/// Set socket option
	pub fn setsockopt<T>(&self, level: libc::c_int, optname: libc::c_int, optval: &T) -> Result<()> {
		unsafe {
			return match libc::setsockopt(self.0, level, optname, transmute(optval), size_of::<T>() as libc::socklen_t) {
				0 => Ok(()),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Get socket option
	pub fn getsockopt<T>(&self, level: libc::c_int, optname: libc::c_int) -> Result<T> {
		unsafe {
			let mut val: T = zeroed();
			let mut len = size_of::<T>() as libc::socklen_t;
			return match getsockopt(self.0, level, optname, transmute(&mut val), &mut len) {
				0 => Ok(val),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Get SCTP socket option
	pub fn sctp_opt_info<T>(&self, optname: libc::c_int, assoc: sctp_sys::sctp_assoc_t) -> Result<T> {
		unsafe {
			let mut val: T = zeroed();
			let mut len = size_of::<T>() as libc::socklen_t;
			return match sctp_sys::sctp_opt_info(self.0, assoc, optname, transmute(&mut val), &mut len) {
				0 => Ok(val),
				_ => Err(Error::last_os_error())
			};
		}
	}
	
	/// Try to clone this socket
	pub fn try_clone(&self) -> Result<SctpSocket> {
		unsafe {
			let new_sock = try!(check_socket(libc::dup(self.0 as i32) as SOCKET));
			return Ok(SctpSocket(new_sock));
		}
	}
}

impl Read for SctpSocket {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
		return self.recv(buf);
	}
}

impl Write for SctpSocket {
	fn write(&mut self, buf: &[u8]) -> Result<usize> {
		return self.send(buf);
	}
	
	fn flush(&mut self) -> Result<()> {
		return Ok(());
	}
}

#[cfg(target_os="windows")]
impl AsRawHandle for SctpSocket {
	fn as_raw_handle(&self) -> RawHandle {
		return self.0 as RawHandle;	
	}
}

#[cfg(target_os="windows")]
impl FromRawHandle for SctpSocket {
	unsafe fn from_raw_handle(hdl: RawHandle) -> SctpSocket {
		return SctpSocket(hdl as SOCKET);
	}
}

#[cfg(target_os="linux")]
impl AsRawFd for SctpSocket {
	fn as_raw_fd(&self) -> RawFd {
		return self.0;	
	}
}

#[cfg(target_os="linux")]
impl FromRawFd for SctpSocket {
	unsafe fn from_raw_fd(fd: RawFd) -> SctpSocket {
		return SctpSocket(fd);
	}
}

impl Drop for SctpSocket {
	fn drop(&mut self) {
		unsafe { closesocket(self.0) };
	}
}