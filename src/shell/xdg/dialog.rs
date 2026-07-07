use crate::reexports::client::{protocol::wl_compositor::WlCompositor, Proxy, QueueHandle};
use crate::reexports::client::{protocol::wl_surface, Connection, Dispatch};
use crate::shell::WaylandSurface;
use crate::{
    compositor::{Surface, SurfaceData},
    globals::ProvidesBoundGlobal,
};
use crate::{error::GlobalError, shell::xdg::XdgShellSurface};
use std::sync::{Arc, Weak};
use wayland_protocols::xdg::{dialog::v1::client::xdg_dialog_v1, shell::client::xdg_surface};
use wayland_protocols::xdg::{
    dialog::v1::client::xdg_dialog_v1::XdgDialogV1, shell::client::xdg_wm_base,
};
use wayland_protocols::xdg::{dialog::v1::client::xdg_wm_dialog_v1, shell::client::xdg_toplevel};

/// Handler for toplevel operations on a [`Dialog`]
pub trait DialogHandler: Sized {
    fn configure(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        window: &Dialog,
        configure: DialogConfigure,
        serial: u32,
    );
}

#[derive(Debug, Clone)]
pub struct Dialog {
    inner: Arc<DialogInner>,
}

#[derive(Debug)]
pub struct DialogData(pub(crate) Weak<DialogInner>);

#[derive(Debug)]
pub(crate) struct DialogInner {
    pub surface: XdgShellSurface,
    pub xdg_toplevel: xdg_toplevel::XdgToplevel,
    pub xdg_dialog: XdgDialogV1,
}

impl Dialog {
    pub fn new<D, GLOBAL>(
        parent: &xdg_toplevel::XdgToplevel,
        qh: &QueueHandle<D>,
        // TODO: is 6 correct?
        compositor: &impl ProvidesBoundGlobal<WlCompositor, 6>,
        wm: &GLOBAL,
    ) -> Result<Self, GlobalError>
    where
        D: Dispatch<wl_surface::WlSurface, SurfaceData>
            + Dispatch<xdg_surface::XdgSurface, DialogData>
            + Dispatch<xdg_dialog_v1::XdgDialogV1, DialogData>
            + Dispatch<xdg_toplevel::XdgToplevel, DialogData>
            + 'static,
        GLOBAL: ProvidesBoundGlobal<xdg_wm_dialog_v1::XdgWmDialogV1, 1>
            + ProvidesBoundGlobal<xdg_wm_base::XdgWmBase, 5>,
    {
        let surface = Surface::new(compositor, qh)?;
        let dialog = Self::from_surface(surface, parent, qh, wm)?;
        dialog.wl_surface().commit();
        Ok(dialog)
    }

    pub fn from_surface<D, GLOBAL>(
        surface: impl Into<Surface>,
        parent: &xdg_toplevel::XdgToplevel,
        qh: &QueueHandle<D>,
        wm_base: &GLOBAL,
    ) -> Result<Self, GlobalError>
    where
        D: Dispatch<xdg_surface::XdgSurface, DialogData>
            + Dispatch<xdg_dialog_v1::XdgDialogV1, DialogData>
            + Dispatch<xdg_toplevel::XdgToplevel, DialogData>
            + 'static,
        GLOBAL: ProvidesBoundGlobal<xdg_wm_dialog_v1::XdgWmDialogV1, 1>
            + ProvidesBoundGlobal<xdg_wm_base::XdgWmBase, 5>,
    {
        let surface = surface.into();
        let wm_dialog: xdg_wm_dialog_v1::XdgWmDialogV1 = wm_base.bound_global()?;
        let wm_base: xdg_wm_base::XdgWmBase = wm_base.bound_global()?;

        // Freeze the queue during the creation of the Arc to avoid a race between events on the
        // new objects being processed and the Weak in the PopupData becoming usable.
        let freeze = qh.freeze();

        let inner = Arc::new_cyclic(|weak| {
            let xdg_surface =
                wm_base.get_xdg_surface(surface.wl_surface(), qh, DialogData(weak.clone()));
            let surface = XdgShellSurface { surface, xdg_surface };
            let xdg_toplevel = surface.xdg_surface.get_toplevel(qh, DialogData(weak.clone()));
            xdg_toplevel.set_parent(Some(parent));
            let xdg_dialog = wm_dialog.get_xdg_dialog(&xdg_toplevel, qh, DialogData(weak.clone()));

            DialogInner { surface, xdg_toplevel, xdg_dialog }
        });
        drop(freeze);
        let dialog = Dialog { inner };
        Ok(dialog)
    }

    pub fn from_xdg_toplevel(toplevel: &xdg_toplevel::XdgToplevel) -> Option<Dialog> {
        toplevel.data::<DialogData>().and_then(|data| data.dialog())
    }

    pub fn from_xdg_surface(surface: &xdg_surface::XdgSurface) -> Option<Dialog> {
        surface.data::<DialogData>().and_then(|data| data.dialog())
    }

    pub fn xdg_dialog(&self) -> &XdgDialogV1 {
        &self.inner.xdg_dialog
    }

    pub fn xdg_shell_surface(&self) -> &XdgShellSurface {
        &self.inner.surface
    }

    pub fn xdg_toplevel(&self) -> &xdg_toplevel::XdgToplevel {
        &self.inner.xdg_toplevel
    }

    pub fn xdg_surface(&self) -> &xdg_surface::XdgSurface {
        self.inner.surface.xdg_surface()
    }

    pub fn wl_surface(&self) -> &wl_surface::WlSurface {
        self.inner.surface.wl_surface()
    }

    pub fn set_modal(&self, modal: bool) {
        if modal {
            self.inner.xdg_dialog.set_modal();
        } else {
            self.inner.xdg_dialog.unset_modal();
        }
    }
}

impl WaylandSurface for Dialog {
    fn wl_surface(&self) -> &wl_surface::WlSurface {
        self.wl_surface()
    }
}

impl DialogData {
    /// Get a new handle to the Dialog
    ///
    /// This returns `None` if the dialog has been destroyed.
    pub fn dialog(&self) -> Option<Dialog> {
        let inner = self.0.upgrade()?;
        Some(Dialog { inner })
    }
}

impl Drop for DialogInner {
    fn drop(&mut self) {
        self.xdg_dialog.destroy();
        self.xdg_toplevel.destroy();
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DialogConfigure {}

impl<D> Dispatch<xdg_surface::XdgSurface, DialogData, D> for DialogData
where
    D: Dispatch<xdg_surface::XdgSurface, DialogData> + DialogHandler,
{
    fn event(
        data: &mut D,
        xdg_surface: &xdg_surface::XdgSurface,
        event: <xdg_surface::XdgSurface as wayland_client::Proxy>::Event,
        _: &DialogData,
        conn: &Connection,
        qhandle: &QueueHandle<D>,
    ) {
        if let Some(dialog) = Dialog::from_xdg_surface(xdg_surface) {
            match event {
                xdg_surface::Event::Configure { serial } => {
                    xdg_surface.ack_configure(serial);

                    let configure = DialogConfigure {};
                    DialogHandler::configure(data, conn, qhandle, &dialog, configure, serial)
                }
                _ => unreachable!(),
            }
        }
    }
}

impl<D> Dispatch<XdgDialogV1, DialogData, D> for DialogData
where
    D: Dispatch<XdgDialogV1, DialogData>,
{
    fn event(
        _state: &mut D,
        _proxy: &XdgDialogV1,
        _event: <XdgDialogV1 as wayland_client::Proxy>::Event,
        _data: &DialogData,
        _conn: &Connection,
        _qhandle: &QueueHandle<D>,
    ) {
    }
}

impl<D> Dispatch<xdg_toplevel::XdgToplevel, DialogData, D> for DialogData
where
    D: Dispatch<xdg_toplevel::XdgToplevel, DialogData>,
{
    fn event(
        _state: &mut D,
        _proxy: &xdg_toplevel::XdgToplevel,
        _event: <xdg_toplevel::XdgToplevel as wayland_client::Proxy>::Event,
        _data: &DialogData,
        _conn: &Connection,
        _qhandle: &QueueHandle<D>,
    ) {
    }
}

#[macro_export]
macro_rules! delegate_xdg_dialog_v1 {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        $crate::reexports::client::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::reexports::protocols::xdg::dialog::v1::client::xdg_dialog_v1::XdgDialogV1: $crate::shell::xdg::dialog::DialogData
        ] => $crate::shell::xdg::dialog::DialogData);
        $crate::reexports::client::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::reexports::protocols::xdg::shell::client::xdg_surface::XdgSurface: $crate::shell::xdg::dialog::DialogData
        ] => $crate::shell::xdg::dialog::DialogData);
        $crate::reexports::client::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::reexports::protocols::xdg::shell::client::xdg_toplevel::XdgToplevel: $crate::shell::xdg::dialog::DialogData
        ] => $crate::shell::xdg::dialog::DialogData);
    };
}
